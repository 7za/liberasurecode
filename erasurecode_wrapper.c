#include <stdlib.h>
#include <liberasurecode/erasurecode.h>
#include <liberasurecode/erasurecode_helpers_ext.h>
#include <liberasurecode/erasurecode_postprocessing.h>

int encode_internal(int desc, char *k_ref[], char *m_ref[],
        size_t chunk_size, size_t tot_len, int k, int m)
{
  ec_backend_t ec = liberasurecode_backend_instance_get_by_desc(desc);
  int ret = ec->common.ops->encode(ec->desc.backend_desc, k_ref, m_ref, chunk_size);
  if (ret < 0) {
    fprintf(stderr, "error encode ret = %d\n", ret);
    return -1;
  }
  // fill the headers with true len, fragment len ....
  ret = finalize_fragments_after_encode(ec, k, m, chunk_size, tot_len, k_ref, m_ref);
  if (ret < 0) {
    fprintf(stderr, "error encode ret = %d\n", ret);
    return -1;
  }
  fprintf(stderr, "IN C : %p\n", m_ref[0]);
  return 0;
}
