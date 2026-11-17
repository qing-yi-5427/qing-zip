#pragma once
#include <array>
#include <cstdint>
#include <cstring>
#include <vector>

// Only ZIP metadata properties use this bounded wire-write optimization. The
// byte stream remains the original kind(9), little-endian length, then payload.
namespace {
constexpr size_t zip_list_packet_limit = 4096;
template<class Packed, class Fallback>
void zip_list_send_property(const std::vector<uint8_t> &payload,
                            Packed &&packed, Fallback &&fallback) {
  if (payload.size() > zip_list_packet_limit - 5) {
    fallback(); // Keep the original frame limit and send path for large text.
    return;
  }
  std::array<uint8_t, zip_list_packet_limit> packet;
  packet[0] = 9;
  const auto length = static_cast<uint32_t>(payload.size());
  for (unsigned i = 0; i < 4; ++i) packet[1 + i] = uint8_t(length >> (8 * i));
  if (length) std::memcpy(packet.data() + 5, payload.data(), length);
  // Every transmitted byte was set above; the unused stack tail is never sent.
  packed(packet.data(), size_t(length) + 5);
}
}
