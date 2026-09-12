// placement_bench.c - what correct NUMA placement is worth, against honest baselines.
//
// One kernel, three placements, the same total bytes read by the same number of
// threads in every case. Only where the pages live changes.
//
//   first-touch  one buffer, faulted by the main thread, so every page lands on
//                that thread's node. Every thread on the other socket then reads
//                across the interconnect. This is what a program gets by writing
//                nothing -- allocate up front, parallel-for later -- and it is
//                the shape the naive baseline has to be, because it is the one
//                real programs accidentally write.
//
//   interleaved  numa_alloc_interleaved. Pages alternate between nodes, so about
//                half of every thread's reads are remote regardless of where it
//                runs. This is the baseline that matters: it is what `numactl
//                --interleave=all` gives for free, it needs no source change,
//                and beating it is the only thing that makes placement worth
//                expressing in a language.
//
//   placed       one buffer per node, and each thread reads the buffer on the
//                node it is running on -- decided at run time from
//                numa_node_of_cpu(sched_getcpu()), so it is right whatever the
//                CPU numbering looks like. Every read is local.
//
// Comparing against first-touch alone would be a strawman: it is the worst case,
// and the kernel's own balancer often repairs it. Interleaved is the number to
// beat.
#define _GNU_SOURCE
#include <numa.h>
#include <numaif.h>
#include <omp.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#include <time.h>

static double now_s(void) {
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return ts.tv_sec + ts.tv_nsec * 1e-9;
}

// Sum `n` u64 from `p`, split across the calling team. The kernel is deliberately
// trivial: one dependent-free load per element, so the time is the memory's and
// not the arithmetic's.
static uint64_t sum_range(const uint64_t *p, size_t n) {
  uint64_t s = 0;
#pragma omp for schedule(static)
  for (size_t i = 0; i < n; i++) {
    s += p[i];
  }
  return s;
}

int main(int argc, char **argv) {
  size_t mib = argc > 1 ? (size_t)strtoull(argv[1], NULL, 10) : 8192;
  int reps = argc > 2 ? atoi(argv[2]) : 5;

  if (numa_available() < 0) {
    fprintf(stderr, "libnuma reports no NUMA support on this machine\n");
    return 1;
  }
  int nodes = numa_max_node() + 1;
  if (nodes < 2) {
    fprintf(stderr, "%d NUMA node(s): nothing to place\n", nodes);
    return 1;
  }

  size_t bytes = mib * 1024 * 1024;
  size_t n = bytes / sizeof(uint64_t);
  size_t half = n / 2;

  // first-touch: one buffer, every page faulted by this thread.
  uint64_t *ft = aligned_alloc(4096, bytes);
  for (size_t i = 0; i < n; i++) ft[i] = i;

  // interleaved: libnuma stripes the pages across every node.
  uint64_t *il = numa_alloc_interleaved(bytes);
  for (size_t i = 0; i < n; i++) il[i] = i;

  // placed: one buffer per node, each faulted on the node it belongs to.
  uint64_t *per_node[2];
  for (int nd = 0; nd < 2; nd++) {
    per_node[nd] = numa_alloc_onnode(half * sizeof(uint64_t), nd);
    for (size_t i = 0; i < half; i++) per_node[nd][i] = i;
  }

  int threads = 0;
#pragma omp parallel
  {
#pragma omp master
    threads = omp_get_num_threads();
  }

  printf("  %zu MiB, %d threads, %d nodes, best of %d\n\n", mib, threads, nodes,
         reps);

  // Which node each thread turned out to be on, filled in on every placed pass.
  int *node_of = calloc((size_t)threads, sizeof(int));
  if (!node_of) return 1;

  double best[3] = {1e30, 1e30, 1e30};
  volatile uint64_t sink = 0;

  for (int r = 0; r < reps; r++) {
    // 0: first-touch   1: interleaved
    for (int which = 0; which < 2; which++) {
      const uint64_t *buf = which == 0 ? ft : il;
      double t0 = now_s();
      uint64_t total = 0;
#pragma omp parallel reduction(+ : total)
      { total += sum_range(buf, n); }
      double dt = now_s() - t0;
      sink += total;
      if (dt < best[which]) best[which] = dt;
    }

    // 2: placed -- each thread reads only the buffer on its own node.
    //
    // The work split has to be by hand. An `omp for` over `half` would divide
    // that half across ALL threads, so each node's buffer would be covered by
    // every thread rather than by its own, and the run would touch half the
    // bytes of the other two configurations while being timed as though it had
    // touched them all. That inflated this row to 484 GB/s on a machine whose
    // two sockets peak at 282 -- the giveaway being a rate the hardware cannot
    // produce, not anything in the shape of the code.
    //
    // So: each thread finds its node, its rank among the threads on that node,
    // and takes a contiguous slice of that node's buffer. The union is each
    // buffer exactly once, which is the same total bytes as the other two.
    {
      double t0 = now_s();
      uint64_t total = 0;
#pragma omp parallel reduction(+ : total)
      {
        int me = omp_get_thread_num();
        node_of[me] = numa_node_of_cpu(sched_getcpu());
        if (node_of[me] < 0 || node_of[me] > 1) node_of[me] = 0;
#pragma omp barrier
        int nd = node_of[me], rank = 0, cnt = 0;
        for (int i = 0; i < threads; i++) {
          if (node_of[i] == nd) {
            if (i < me) rank++;
            cnt++;
          }
        }
        uint64_t s = 0;
        if (cnt > 0) {
          size_t chunk = half / (size_t)cnt;
          size_t lo = (size_t)rank * chunk;
          size_t hi = (rank == cnt - 1) ? half : lo + chunk;
          const uint64_t *buf = per_node[nd];
          for (size_t i = lo; i < hi; i++) {
            s += buf[i];
          }
        }
        total += s;
      }
      double dt = now_s() - t0;
      sink += total;
      if (dt < best[2]) best[2] = dt;
    }
  }
  (void)sink;

  const char *name[3] = {"first-touch (one node)", "interleaved (numactl)",
                         "placed (per node)"};
  double gbs[3];
  for (int i = 0; i < 3; i++) gbs[i] = (double)bytes / best[i] / 1e9;

  for (int i = 0; i < 3; i++) {
    printf("  %-24s %8.1f GB/s   %7.1f ms\n", name[i], gbs[i], best[i] * 1e3);
  }
  printf("\n  placed / interleaved   %.2fx   <- the number that matters\n",
         gbs[2] / gbs[1]);
  printf("  placed / first-touch   %.2fx\n", gbs[2] / gbs[0]);

  free(node_of);
  free(ft);
  numa_free(il, bytes);
  for (int nd = 0; nd < 2; nd++) numa_free(per_node[nd], half * sizeof(uint64_t));
  return 0;
}
