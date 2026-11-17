// From obz-rs in an x64 Native Tools command prompt:
// cl /nologo /std:c++20 /EHsc /MT /O2 tests\native\zip_list_packet_test.cpp /Fe:zip_list_packet_test.exe
// .\zip_list_packet_test.exe
// Keep assertions active even when an embedding build defines NDEBUG.
#ifdef NDEBUG
#undef NDEBUG
#endif
// Pure byte tests of the exact production helper; no pipes, workers or files.
#include "../../native/nana/zip_list_packet.h"
#include <algorithm>
#include <cassert>
#include <iostream>
#include <stdexcept>

using Bytes = std::vector<uint8_t>;
Bytes reference(const Bytes &payload) {
  Bytes wire{9};
  uint32_t length = uint32_t(payload.size());
  for (unsigned i = 0; i < 4; ++i) wire.push_back(uint8_t(length >> (8 * i)));
  wire.insert(wire.end(), payload.begin(), payload.end());
  return wire;
}
int main() {
  static_assert(zip_list_packet_limit == 4096);
  unsigned cases = 0;
  for (size_t length : {size_t(0), size_t(1), size_t(4091), size_t(4092), size_t(65580)}) {
    Bytes payload(length);
    for (size_t i = 0; i < length; ++i) payload[i] = uint8_t(i * 17);
    for (size_t chunk : {size_t(1), size_t(3), size_t(4096)}) {
      Bytes actual; unsigned packed_calls = 0, fallback_calls = 0;
      auto short_write = [&](const uint8_t *p, size_t n) {
        while (n) {
          size_t amount = std::min(chunk, n);
          actual.insert(actual.end(), p, p + amount);
          p += amount; n -= amount;
        }
      };
      zip_list_send_property(payload,
        [&](const uint8_t *p, size_t n) {
          ++packed_calls;
          assert(n == length + 5 && n <= zip_list_packet_limit);
          short_write(p, n);
        },
        [&]() {
          ++fallback_calls;
          const auto wire = reference(payload);
          short_write(wire.data(), wire.size());
        });
      assert(actual == reference(payload)); // No uninitialized stack tail sent.
      assert(packed_calls == unsigned(length <= 4091));
      assert(fallback_calls == unsigned(length > 4091));
      ++cases;
    }
  }
  for (size_t length : {size_t(0), size_t(4091), size_t(4092)}) {
    bool caught = false;
    try {
      zip_list_send_property(Bytes(length),
        [](const uint8_t *, size_t) { throw std::runtime_error("memory sink stopped"); },
        []() { throw std::runtime_error("memory sink stopped"); });
    } catch (const std::runtime_error &e) {
      caught = std::strcmp(e.what(), "memory sink stopped") == 0;
    }
    assert(caught); ++cases;
  }
  std::cout << "PASS " << cases << " pure-memory cases; 0/4091/4092 boundaries, byte equivalence, simulated short writes, exception propagation\n";
}
