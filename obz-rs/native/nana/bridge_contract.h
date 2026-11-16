#pragma once
#include <windows.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

enum { QZ_TRACE_ENCRYPTED_HEADER = 25, QZ_TRACE_THREADS_APPLIED = 26 };

enum QzSelectedOutcome {
  QZ_SELECTED_FAILED = 1,         // No CLI retry, publication, recycle, or success.
  QZ_SELECTED_DECODED_PRIVATE = 2, // Still needs worker exit + host verification.
  QZ_SELECTED_UNSUPPORTED_LAYOUT = 3 // Valid feature identified before any manifest acknowledgement/output.
};

struct QzSelectedTime {
  uint64_t filetime_100ns;
  uint32_t present;
  uint16_t precision;    // Official PROPVARIANT.wReserved1, not locale text.
  uint16_t sub100ns;     // Preserve official finer-than-FILETIME information.
};

struct QzSelectedItem {
  const wchar_t *name;   // Borrowed UTF-16, explicit length, no embedded NUL.
  uint32_t name_units;
  uint32_t index;
  uint64_t size;
  uint32_t is_dir;
  uint32_t attrs_present;
  uint32_t attrs;
  uint32_t crc_present;  // Informational. CRC/auth are checked by official DLL.
  uint32_t crc;
  uint32_t encrypted; // Fits former padding; serialized only in verification protocol v2.
  struct QzSelectedTime ctime, atime, mtime;
};

struct QzSelectedVolume {
  const wchar_t *approved_leaf;
  uint32_t leaf_units;
  uint32_t reserved;
  uint64_t fixed_size;
  // Its implicit index identifies a host-owned pinned source + ancestor guards.
  // The bridge has NO callback accepting a path to open or enumerate.
};

struct QzSelectedHost {
  uint32_t abi_version; // Must be 1.
  uint32_t struct_bytes;
  void *context;
  // Host must retain pins until worker exit, not just until this function returns.
  HRESULT (__stdcall *read_volume_at)(void *, uint32_t volume, uint64_t offset,
                                     void *buffer, uint32_t size, uint32_t *read);
  // Whole manifest only; reuse expected_tree/validate_name/selected() semantics.
  // On S_OK fill every selected[i] with exactly 0 or 1. No output creation here.
  // Borrowed pointers expire at return: deep-copy names/time/selection; retain no
  // pointer to the items or selection buffer and never mutate it asynchronously.
  HRESULT (__stdcall *accept_manifest)(void *, const struct QzSelectedItem *,
                                      uint32_t count, uint8_t *selected);
  // After full manifest acknowledgement, create only selected directories and
  // required ancestors inside the private stage (including empty directories).
  // Never touches the final destination. Host cached the acknowledged manifest.
  HRESULT (__stdcall *prepare_private_tree)(void *);
  // Called only after accept_manifest and prepare_private_tree succeed. Host uses create_staged_output
  // under an already atomically private root, never the final destination.
  HRESULT (__stdcall *open_private_output)(void *, uint32_t item, uint64_t *token);
  HRESULT (__stdcall *write_private_output)(void *, uint64_t token,
                                           const void *, uint32_t, uint32_t *);
  HRESULT (__stdcall *close_private_output)(void *, uint64_t token);
  // Thread-safe. Nonzero means cancelled; no blocking UI/password interaction.
  int32_t (__stdcall *cancelled)(void *);
  void (__stdcall *progress)(void *, uint64_t decoded, uint64_t total);
  // The host retains official per-item/block error codes for the terminal receipt.
  HRESULT (__stdcall *trace)(void *, uint32_t event, uint32_t index,
                            int32_t mode, int32_t result, uint64_t value);
};

struct QzSelectedRequest {
  uint32_t abi_version;
  uint32_t struct_bytes;
  const wchar_t *sealed_runtime_directory; // Absolute host-validated pinned graph.
  const struct QzSelectedVolume *volumes;
  uint32_t volume_count;
  uint32_t reserved;
  const struct QzSelectedHost *host;
};

struct QzSelectedReceipt {
  uint32_t outcome;
  uint32_t item_count;
  uint32_t completion_count;
  uint32_t item_errors;
  uint32_t block_errors;
  uint32_t streams_open;
  uint64_t decoded_bytes;
  uint64_t selected_bytes;
  HRESULT result;
  uint32_t reserved;
  // NO paths/passwords. First failing category only, bounded UTF-8 ASCII text.
  char reason[192];
};

// ALL failures, including active/unknown policies, are terminal QZ_SELECTED_FAILED.
// There is no generic CLI fallback from this worker. The host must not infer
// that S_OK authorizes publishing.
HRESULT __stdcall qz_extract_selected(
    const struct QzSelectedRequest *, struct QzSelectedReceipt *);

// Single archive worker only. 0 keeps the official handler default. The old
// selected ABI and request struct remain unchanged.
HRESULT __stdcall qz_extract_single_with_threads(
    const struct QzSelectedRequest *, struct QzSelectedReceipt *, uint32_t);

// Internal worker verification mode: all-item decode/CRC with explicit zero
// output permitted. Ordinary extraction continues to reject empty selections.
HRESULT __stdcall qz_verify_single_with_threads(
    const struct QzSelectedRequest *, struct QzSelectedReceipt *, uint32_t, const wchar_t *, uint32_t, uint32_t, uint32_t);

// Borrowed only until return. Decoders may request a caller-owned BSTR.
HRESULT __stdcall qz_extract_selected_with_password(
    const struct QzSelectedRequest *, struct QzSelectedReceipt *, const wchar_t *, uint32_t);

HRESULT __stdcall qz_extract_selected_reusing_verified(const struct QzSelectedRequest *, struct QzSelectedReceipt *, const wchar_t *, uint32_t, uint32_t);

// Read-only ZIP listing. Property ids are stable protocol ordinals, not SDK ids.
// No output or volume-discovery callback exists in this capability surface.
struct QzZipListProperty {
  uint32_t id, type; // 0 empty, 1 number, 2 bool, 3 UTF-16, 4 FILETIME.
  uint64_t number;
  const wchar_t *text;
  uint32_t text_units;
  uint16_t precision, sub100ns, reserved3, padding;
};
struct QzZipListHost {
  uint32_t abi_version, struct_bytes;
  void *context;
  HRESULT (__stdcall *read_at)(void *, uint32_t, uint64_t, void *, uint32_t, uint32_t *);
  int32_t (__stdcall *cancelled)(void *);
  // index == UINT32_MAX carries archive diagnostics before item metadata.
  HRESULT (__stdcall *property)(void *, uint32_t index, const struct QzZipListProperty *);
};
struct QzZipListRequest {
  uint32_t abi_version, struct_bytes;
  const wchar_t *runtime;
  uint64_t source_size, local_offset;
  const struct QzZipListHost *host;
};
HRESULT __stdcall qz_list_zip_at(const struct QzZipListRequest *, uint32_t *item_count);

#ifdef __cplusplus
}
#endif
