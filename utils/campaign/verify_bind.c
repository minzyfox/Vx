// verify_bind.c - does a NUMA bind actually place the pages, and can the kernel tell you?
//
//   gcc -O2 -o verify_bind verify_bind.c && ./verify_bind
//
// Two questions, and the gap between them is the point.
//
// The first is whether `mbind` did what it was asked. This replicates exactly what
// `vx_numa_alloc` in runtime/host_dispatch_common.h does -- mmap, then bind the mapping to one
// node through the syscall -- and then asks `move_pages()` which node each page actually ended up
// on. That is the kernel's own answer, and a stronger check than the page accounting in
// /proc/PID/numa_maps.
//
// The second is whether that answer means anything, and on a virtualized instance it does not.
// Measured on two machines:
//
//   c5.metal      64/64 pages on the requested node, and a remote read costs 2.35x a local one
//   c4.8xlarge    64/64 pages on the requested node, and a remote read costs the same as a local
//                 one, because the "nodes" are not backed by physical locality at all
//
// Identical output, opposite realities. The kernel is not lying in either case: it answers
// truthfully about GUEST nodes, and on the c4 the guest nodes are the thing that is fiction. So no
// placement query available inside a guest can separate a real placement from a synthetic one --
// which is why utils/campaign/run_numa_probe.sh decides with a bandwidth measurement instead, and
// refuses to report a ratio from a machine whose binds confine nothing.
//
// Use this to confirm the binding path works. Use the probe to confirm the machine is real. They
// are different questions and this one cannot answer the second.
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <unistd.h>

#ifndef MPOL_BIND
#define MPOL_BIND 2
#endif
#ifndef MPOL_MF_MOVE
#define MPOL_MF_MOVE (1 << 1)
#endif

// Mirrors VX_NUMA_HEADER in runtime/host_dispatch_common.h: one page in front of the allocation,
// holding the mapping's length so the free can unmap it.
#define VX_NUMA_HEADER 4096

static void *vx_numa_alloc(size_t bytes, int node, int *bound) {
  *bound = 0;
  size_t total = bytes + VX_NUMA_HEADER;
  void *base = mmap(NULL, total, PROT_READ | PROT_WRITE,
                    MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
  if (base == MAP_FAILED) {
    return NULL;
  }
  unsigned long mask[999 / (8 * sizeof(unsigned long)) + 1] = {0};
  mask[node / (int)(8 * sizeof(unsigned long))] |=
      1UL << (node % (int)(8 * sizeof(unsigned long)));
  long rc = syscall(__NR_mbind, base, total, MPOL_BIND, mask,
                    (unsigned long)(sizeof(mask) * 8), MPOL_MF_MOVE);
  *bound = (rc == 0);
  *(size_t *)base = total;
  return (char *)base + VX_NUMA_HEADER;
}

int main(int argc, char **argv) {
  int nodes = argc > 1 ? atoi(argv[1]) : 2;
  // `atoi` answers 0 for anything unparseable, and a loop over zero nodes exits cleanly having
  // probed nothing -- a silent success that looks exactly like a real one.
  if (nodes < 1) {
    fprintf(stderr, "usage: verify_bind [node count]   (got '%s')\n",
            argc > 1 ? argv[1] : "");
    return 2;
  }
  size_t bytes = 64ul << 20;

  for (int want = 0; want < nodes; want++) {
    int bound = 0;
    char *p = (char *)vx_numa_alloc(bytes, want, &bound);
    if (!p) {
      printf("  node %d: mmap failed\n", want);
      continue;
    }
    // The pages are not committed until touched, and an untouched mapping has no node to report.
    memset(p, 1, bytes);

    const int probes = 64;
    void *pages[64];
    int status[64];
    for (int i = 0; i < probes; i++) {
      pages[i] = p + ((size_t)i * bytes / probes);
    }
    long rc = syscall(__NR_move_pages, 0, probes, pages, NULL, status, 0);

    int ok = 0, other = 0;
    for (int i = 0; i < probes; i++) {
      if (status[i] == want) {
        ok++;
      } else {
        other++;
      }
    }
    printf("  asked node %d: bind %s, kernel reports %d/%d pages on node %d"
           " (%d elsewhere)%s\n",
           want, bound ? "ok" : "FAILED", ok, probes, want, other,
           rc ? " [move_pages query failed]" : "");
  }

  printf("\n  A clean result here means the binding path works. It does NOT mean the\n"
         "  machine has real NUMA -- run utils/campaign/run_numa_probe.sh for that.\n");
  return 0;
}
