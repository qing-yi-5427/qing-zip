#ifdef NDEBUG
#undef NDEBUG
#endif
#include "../src/launch.h"
#include <shellapi.h>
#include <cassert>
#include <fstream>
#include <iterator>

int wmain(int argc, wchar_t** argv) {
    assert(argc == 2);
    const std::vector<std::wstring> paths = {L"C:\\普通目录\\测试.zip", L"C:\\with spaces\\",
        L"C:\\a&b;$(name).zip", L"\\\\server\\share\\file.zip", L"a\"b\\", L""};
    std::wstring command = qing::quote(L"C:\\Program Files\\qing-zip\\qing-zip.exe");
    for (const auto& path : paths) { command += L" " + qing::quote(path); }
    int count = 0; PWSTR* args = CommandLineToArgvW(command.c_str(), &count);
    assert(args && count == static_cast<int>(paths.size()) + 1);
    for (size_t index = 0; index < paths.size(); ++index) { assert(args[index + 1] == paths[index]); }
    LocalFree(args);
    assert(qing::supported_path(L"C:\\normal folder\\"));
    assert(qing::supported_path(L"\\\\server\\share\\a.zip"));
    for (const auto* rejected : {L"relative.zip", L"C:relative.zip", L"\\\\.\\PhysicalDrive0", L"\\\\?\\C:\\a.zip", L"https://example/a", L"C:\\a\nb.zip"}) {
        assert(!qing::supported_path(rejected));
    }
    assert(qing::archive_path(L"C:\\a.ZIP"));
    for (const auto* ext : {L".brotli", L".lz5", L".liz", L".lizard", L".ZSTD",
                            L".wasm", L".romfs", L".ufs", L".ufs2", L".zealfs"}) {
        assert(qing::archive_path(std::wstring(L"C:\\sample") + ext));
    }
    assert(qing::archive_path(L"C:\\archive.7z.001"));
    assert(!qing::archive_path(L"C:\\archive.7z.002"));
    assert(!qing::archive_path(L"C:\\image.png"));
    const auto output = std::filesystem::temp_directory_path() / (L"qing-shell-args-" + std::to_wstring(GetCurrentProcessId()));
    assert(!std::filesystem::exists(output));
    assert(SetEnvironmentVariableW(L"QING_SHELL_TEST_OUTPUT", output.c_str()));
    const std::vector<std::wstring> launch_paths = {L"C:\\中文目录\\a & b;$(test).zip", L"C:\\trailing slash\\", L"\\\\server\\share\\b.7z"};
    for (const auto action : {qing::Action::Open, qing::Action::Compress, qing::Action::SmartExtract, qing::Action::ExtractHere}) {
        assert(qing::launch(argv[1], action, launch_paths) == S_OK);
        for (int attempt = 0; attempt < 500 && !std::filesystem::exists(output); ++attempt) { Sleep(10); }
        assert(std::filesystem::exists(output));
        std::ifstream file(output, std::ios::binary);
        std::string bytes((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
        assert(bytes.size() % sizeof(wchar_t) == 0);
        std::vector<wchar_t> wide(bytes.size() / sizeof(wchar_t));
        memcpy(wide.data(), bytes.data(), bytes.size());
        std::vector<std::wstring> received;
        for (size_t pos = 0; pos < wide.size();) { received.emplace_back(wide.data() + pos); pos += received.back().size() + 1; }
        const size_t offset = action == qing::Action::Open ? 1 : 2;
        assert(received.size() == launch_paths.size() + offset);
        if (offset == 2) { assert(received[1] == std::wstring(qing::argument(action)).substr(1)); }
        for (size_t i = 0; i < launch_paths.size(); ++i) { assert(received[i + offset] == launch_paths[i]); }
        file.close(); assert(std::filesystem::remove(output));
    }
    SetEnvironmentVariableW(L"QING_SHELL_TEST_OUTPUT", nullptr);
    return 0;
}
