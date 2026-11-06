#include "launch.h"
#include <new>

int WINAPI wWinMain(HINSTANCE module, HINSTANCE, PWSTR, int) {
    try {
        std::wstring executable;
        const HRESULT result = qing::app_path(module, executable);
        return FAILED(result) || FAILED(qing::launch(executable, qing::Action::Open, {})) ? 1 : 0;
    } catch (...) { return 1; }
}
