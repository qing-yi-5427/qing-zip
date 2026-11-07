// Launch one archive reader with an explicit standard-I/O handle allowlist.
// Only the selected worker receives additional, individually granted handles.
#include <windows.h>
#include <string>
#include <string_view>
#include <vector>

namespace {
struct Handle {
  HANDLE value{};
  ~Handle() { if (value && value != INVALID_HANDLE_VALUE) CloseHandle(value); }
  HANDLE take() { HANDLE result = value; value = nullptr; return result; }
};
struct Attributes {
  LPPROC_THREAD_ATTRIBUTE_LIST list{};
  ~Attributes() { if (list) DeleteProcThreadAttributeList(list); }
};
}

static DWORD spawn_archive_reader(const wchar_t *exe, const wchar_t *archive, DWORD threads,
                                 HANDLE *process, HANDLE *input, HANDLE *output) noexcept {
  *process = *input = *output = nullptr;
  try {
    Handle child_in, parent_in, child_out, parent_out, child_err;
    if (!CreatePipe(&child_in.value, &parent_in.value, nullptr, 0) ||
        !CreatePipe(&parent_out.value, &child_out.value, nullptr, archive ? 64 * 1024 : 0)) return GetLastError();
    SECURITY_ATTRIBUTES inheritable{sizeof(SECURITY_ATTRIBUTES), nullptr, TRUE};
    child_err.value = CreateFileW(L"NUL", GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE,
                                  &inheritable, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (child_err.value == INVALID_HANDLE_VALUE) return GetLastError();
    if (!SetHandleInformation(child_in.value, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) ||
        !SetHandleInformation(child_out.value, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT)) return GetLastError();

    SIZE_T length = 0;
    InitializeProcThreadAttributeList(nullptr, 2, 0, &length);
    if (!length) return GetLastError();
    std::vector<unsigned char> buffer(length);
    Attributes attributes;
    auto list = reinterpret_cast<LPPROC_THREAD_ATTRIBUTE_LIST>(buffer.data());
    if (!InitializeProcThreadAttributeList(list, 2, 0, &length)) return GetLastError();
    attributes.list = list;
    HANDLE allowed[] = {child_in.value, child_out.value, child_err.value};
    if (!UpdateProcThreadAttribute(list, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, allowed,
                                   sizeof(allowed), nullptr, nullptr)) return GetLastError();
    DWORD child_policy = PROCESS_CREATION_CHILD_PROCESS_RESTRICTED;
    if (!UpdateProcThreadAttribute(list, 0, PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY,
                                   &child_policy, sizeof(child_policy), nullptr, nullptr)) return GetLastError();
    STARTUPINFOEXW startup{};
    startup.StartupInfo.cb = sizeof(startup);
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdInput = child_in.value;
    startup.StartupInfo.hStdOutput = child_out.value;
    startup.StartupInfo.hStdError = child_err.value;
    startup.lpAttributeList = list;
    PROCESS_INFORMATION child{};
    std::wstring command = L"\"" + std::wstring(exe) + L"\"";
    if (archive) {
      // Both are existing Windows file names, so embedded quotes and a
      // trailing directory separator cannot be valid command-line operands.
      std::wstring_view file(archive);
      if (file.empty() || file.find(L'"') != file.npos || file.back() == L'\\')
        return ERROR_INVALID_NAME;
      command += L" x -so -bso0 -bsp0 -y -spd -t7z";
      if (threads) command += L" -mmt=" + std::to_wstring(threads);
      command += L" -- \"" + std::wstring(file) + L"\"";
    } else {
      command += L" --internal-selected-worker";
    }
    if (!CreateProcessW(exe, command.data(), nullptr, nullptr, TRUE,
                        DETACHED_PROCESS | EXTENDED_STARTUPINFO_PRESENT, nullptr, nullptr,
                        &startup.StartupInfo, &child)) return GetLastError();
    CloseHandle(child.hThread);
    *process = child.hProcess; *input = parent_in.take(); *output = parent_out.take();
    return ERROR_SUCCESS;
  } catch (...) { return ERROR_OUTOFMEMORY; }
}

extern "C" DWORD qing_spawn_selected_worker(const wchar_t *exe, HANDLE *process,
                                            HANDLE *input, HANDLE *output) noexcept {
  return spawn_archive_reader(exe, nullptr, 0, process, input, output);
}

extern "C" DWORD qing_spawn_solid_reader(const wchar_t *exe, const wchar_t *archive, DWORD threads,
                                         HANDLE *process, HANDLE *output) noexcept {
  HANDLE input = nullptr;
  auto status = spawn_archive_reader(exe, archive, threads, process, &input, output);
  if (input) CloseHandle(input); // This operation never supplies a password.
  return status;
}
