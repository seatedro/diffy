#include <stdio.h>

/* Prints a greeting. */
static int greet(const char *name) {
    return printf("hello, %s\n", name);
}

int main(void) {
    greet("quark");
    return 0;
}
