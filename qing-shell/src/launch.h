#pragma once
#include <windows.h>
#include <filesystem>
#include <string>
#include <cwctype>
#include <vector>

namespace qing {
enum class Action { Open, Compress, SmartExtract, ExtractHere };
inline const wchar_t* argument(Action action) {
    switch (action) {
    case Action::Compress: return L" --compress-folder";
    case Action::SmartExtract: return L" --smart-extract";
    case Action::ExtractHere: return L" --extract-here";
    default: return L"";
    }
}
inline bool archive_path(const std::wstring& value) {
    auto ext = std::filesystem::path(value).extension().wstring();
    for (auto& ch : ext) { ch = static_cast<wchar_t>(towlower(ch)); }
    for (const auto* supported : {L".zip", L".7z", L".rar", L".zipx", L".tar", L".gz", L".bz2", L".xz", L".tgz", L".tbz", L".tbz2", L".txz", L".iso", L".cab", L".wim", L".lzh", L".lha", L".ace", L".egg", L".alz", L".zst", L".asar", L".br", L".lz4", L".aes", L".lz", L".uu", L".uue", L".xxe", L".pma", L".zpaq", L".daa", L".isz", L".001", L".gzip", L".bzip2"}) {
        if (ext == supported) { return true; }
    }
    return false;
}

// CommandLineToArgvW / MSVC quoting, including trailing backslashes. Never pass
// selection paths through cmd.exe, PowerShell, shell verbs, or registry strings.
inline std::wstring quote(const std::wstring& value) {
    std::wstring out = L"\"";
    size_t slashes = 0;
    for (const wchar_t c : value) {
        if (c == L'\\') { ++slashes; continue; }
        if (c == L'"') { out.append(slashes * 2 + 1, L'\\'); }
        else { out.append(slashes, L'\\'); }
        slashes = 0;
        out.push_back(c);
    }
    out.append(slashes * 2, L'\\');
    out.push_back(L'"');
    return out;
}
inline bool supported_path(const std::wstring& value) {
    if (value.empty() || value.find_first_of(L"\r\n") != std::wstring::npos
        || value.find(L'\0') != std::wstring::npos || value.find(L"://") != std::wstring::npos
        || value.starts_with(L"\\\\?\\") || value.starts_with(L"\\\\.\\")) { return false; }
    return std::filesystem::path(value).is_absolute();
}
inline HRESULT app_path(HMODULE module, std::wstring& path) {
    std::vector<wchar_t> buffer(32768);
    const DWORD count = GetModuleFileNameW(module, buffer.data(), static_cast<DWORD>(buffer.size()));
    if (count == 0 || count >= buffer.size()) { return HRESULT_FROM_WIN32(GetLastError()); }
    // Installer contract: app\shell\{DLL,host.exe}, app\qing-zip.exe.
    path = (std::filesystem::path(std::wstring(buffer.data(), count)).parent_path().parent_path()
        / L"qing-zip.exe").wstring();
    if (path.starts_with(L"\\\\?\\UNC\\")) { path = L"\\\\" + path.substr(8); }
    else if (path.starts_with(L"\\\\?\\")) { path = path.substr(4); }
    return S_OK;
}
inline HRESULT launch(const std::wstring& executable, Action action,
    const std::vector<std::wstring>& paths) {
    if (paths.size() > 128) { return HRESULT_FROM_WIN32(ERROR_TOO_MANY_NAMES); }
    std::wstring command = quote(executable);
    command += argument(action);
    for (const auto& path : paths) {
        if (!supported_path(path)) { return E_INVALIDARG; }
        command += L" " + quote(path);
        if (command.size() >= 30000) { return HRESULT_FROM_WIN32(ERROR_FILENAME_EXCED_RANGE); }
    }
    const DWORD attributes = GetFileAttributesW(executable.c_str());
    if (attributes == INVALID_FILE_ATTRIBUTES || (attributes & FILE_ATTRIBUTE_DIRECTORY) != 0) {
        return HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND);
    }
    STARTUPINFOW startup{}; startup.cb = sizeof(startup);
    PROCESS_INFORMATION process{};
    const auto directory = std::filesystem::path(executable).parent_path().wstring();
    if (!CreateProcessW(executable.c_str(), command.data(), nullptr, nullptr, FALSE,
        CREATE_UNICODE_ENVIRONMENT, nullptr, directory.c_str(), &startup, &process)) {
        return HRESULT_FROM_WIN32(GetLastError());
    }
    CloseHandle(process.hThread);
    CloseHandle(process.hProcess);
    return S_OK;
}
}
