// A stand-in for the `dupin` executable: linked once without Ekuma and once with the archive.
#include <stdio.h>
#include <stdlib.h>

#ifdef WITH_EKUMA
#include "libekuma.h"
#endif

int main(void) {
#ifdef WITH_EKUMA
	char *result = dupin_ekuma_eval("[1, 2, 3].map((n) => n * 2).join(',')");
	puts(result);
	free(result);
#else
	puts("2,4,6");
#endif
	return 0;
}
