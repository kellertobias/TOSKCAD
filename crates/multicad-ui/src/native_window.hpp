#pragma once
#include <cstddef>
#include <cstdint>
namespace multicad::window {
struct NativeTouchFrame;
NativeTouchFrame poll_trackpad(std::size_t view) noexcept;
bool trackpad_contract_test() noexcept;
bool position_buttons(std::size_t view, double header_height) noexcept;
// 0 installed, 1 wrong thread, 2 no window, 3 no delegate, 4 foreign Quit
// implementation, 5 runtime installation failure.
std::uint8_t install_quit_guard(std::size_t view) noexcept;
bool quit_guard_contract_test() noexcept;
}
