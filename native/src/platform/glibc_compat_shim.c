/* Supply libc entry points missing from the webOS SDK. getauxval reads
 * /proc/self/auxv; gettid and sendmmsg call the kernel directly. build.rs links
 * this shim for the ARMv7 TV target. */
#include <stdio.h>
#include <unistd.h>
#include <sys/syscall.h>

unsigned long getauxval(unsigned long type) {
    struct {
        unsigned long a_type;
        unsigned long a_val;
    } aux;
    unsigned long result = 0;

    FILE *f = fopen("/proc/self/auxv", "rb");
    if (!f) {
        return 0;
    }
    while (fread(&aux, sizeof(aux), 1, f) == 1) {
        if (aux.a_type == 0) {
            break; /* AT_NULL terminator */
        }
        if (aux.a_type == type) {
            result = aux.a_val;
            break;
        }
    }
    fclose(f);
    return result;
}

/* The SDK defines SYS_gettid but has no libc wrapper. */
int gettid(void) {
    return (int) syscall(SYS_gettid);
}

/* Batched sendmsg. We deliberately don't declare the real `struct mmsghdr *` type
 * here (this old sysroot's headers may not define it) — syscall(2) forwards the
 * pointer to the kernel untouched, so the exact C type is irrelevant to correctness;
 * only the calling convention (register order or that this shim never dereferences
 * it) needs to match, and it does. */
int sendmmsg(int sockfd, void *msgvec, unsigned int vlen, int flags) {
    return (int) syscall(SYS_sendmmsg, sockfd, msgvec, vlen, flags);
}
