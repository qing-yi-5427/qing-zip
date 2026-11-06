#include <windows.h>
#include <shellapi.h>
#include <filesystem>
#include <fstream>
#include <string>

int WINAPI wWinMain(HINSTANCE, HINSTANCE, PWSTR, int) {
    wchar_t output[32768]{};
    const DWORD length = GetEnvironmentVariableW(L"QING_SHELL_TEST_OUTPUT", output, 32768);
    if (!length || length >= 32768) { return 1; }
    int count{}; PWSTR* args = CommandLineToArgvW(GetCommandLineW(), &count);
    if (!args) { return 2; }
    const auto temporary = std::filesystem::path(std::wstring(output) + L".tmp");
    std::ofstream file(temporary, std::ios::binary);
    for (int index = 0; index < count; ++index) {
        const std::wstring arg = args[index];
        file.write(reinterpret_cast<const char*>(arg.c_str()), static_cast<std::streamsize>((arg.size() + 1) * sizeof(wchar_t)));
    }
    file.close(); LocalFree(args);
    if (!file) { return 3; }
    std::error_code error; std::filesystem::rename(temporary, output, error);
    return error ? 4 : 0;
}
