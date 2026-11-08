// Hosted-only integration probe for the registered classic Explorer verb.
// The selection and launch path are real. This window records the production
// Tauri single-instance protocol instead of displaying a compression dialog.
#include <windows.h>
#include <shlobj.h>
#include <shellapi.h>
#include <wrl/client.h>
#include <algorithm>
#include <chrono>
#include <cstdlib>
#include <filesystem>
#include <iostream>
#include <memory>
#include <stdexcept>
#include <string>
#include <vector>

using Microsoft::WRL::ComPtr;
namespace fs = std::filesystem;
using namespace std::chrono_literals;
namespace {
constexpr wchar_t verb[] = L"qing-zip-compress-files";
constexpr wchar_t sinkClass[] = L"dev.openarchive.app-sic";
constexpr wchar_t sinkTitle[] = L"dev.openarchive.app-siw";
constexpr wchar_t sinkMutex[] = L"dev.openarchive.app-sim";
std::vector<std::string> received;
void check(HRESULT hr, const char* operation) {
    if (FAILED(hr)) { std::cerr << operation << ": 0x" << std::hex << hr << std::dec << '\n'; throw std::runtime_error(operation); }
}
void require(bool ok,const char* reason) { if (!ok) throw std::runtime_error(reason); }
bool envIs(const wchar_t* name,const wchar_t* expected) {
    wchar_t value[64]{};
    const DWORD length=GetEnvironmentVariableW(name,value,static_cast<DWORD>(std::size(value)));
    return length>0 && length<std::size(value) && std::wstring(value)==expected;
}
LRESULT CALLBACK sink(HWND hwnd,UINT msg,WPARAM wp,LPARAM lp) {
    if(msg!=WM_COPYDATA) return DefWindowProcW(hwnd,msg,wp,lp);
    const auto* packet=reinterpret_cast<const COPYDATASTRUCT*>(lp);
    if(!packet || !packet->lpData || !packet->cbData || packet->cbData>262144 || packet->dwData!=1543) return FALSE;
    received.emplace_back(static_cast<const char*>(packet->lpData),packet->cbData);
    return TRUE;
}

void pumpMessages(std::chrono::steady_clock::time_point until) {
    while(std::chrono::steady_clock::now()<until) {
        MsgWaitForMultipleObjects(0,nullptr,FALSE,75,QS_ALLINPUT);
        MSG msg{};
        while(PeekMessageW(&msg,nullptr,0,0,PM_REMOVE)) {TranslateMessage(&msg);DispatchMessageW(&msg);}
    }
}

struct ItemDeleter {using pointer=PIDLIST_RELATIVE;void operator()(pointer p) const {ILFree(p);}};
using Item=std::unique_ptr<ITEMIDLIST,ItemDeleter>;
Item child(IShellFolder* parent,const fs::path& path) {
    auto name=path.filename().wstring();
    PIDLIST_RELATIVE item=nullptr;
    check(parent->ParseDisplayName(nullptr,nullptr,name.data(),nullptr,&item,nullptr),"parse selected item");
    return Item(item);
}
std::vector<std::wstring> arguments(const std::string& raw) {
    std::vector<std::wstring> out;
    for(size_t start=0;start<raw.size();) {
        auto end=raw.find('\0',start);
        if(end==std::string::npos) end=raw.size();
        const auto length=static_cast<int>(end-start);
        const int need=MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,raw.data()+start,length,nullptr,0);
        require(need>0,"invalid or empty single-instance argument");
        std::wstring value(static_cast<size_t>(need),L'\0');
        require(MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,raw.data()+start,length,value.data(),need)==need,"bad UTF-8");
        out.push_back(std::move(value));start=end+1;
    }
    return out;
}
void invokeMenu(HWND hwnd, const fs::path& first, const fs::path& second) {
    PIDLIST_ABSOLUTE rawFolder=nullptr;
    check(SHParseDisplayName(first.parent_path().c_str(),nullptr,&rawFolder,0,nullptr),"parse parent");
    Item folder(rawFolder);
    ComPtr<IShellFolder> desktop,parent;
    check(SHGetDesktopFolder(&desktop),"desktop folder");
    check(desktop->BindToObject(folder.get(),nullptr,IID_PPV_ARGS(&parent)),"bind selection parent");
    auto one=child(parent.Get(),first),two=child(parent.Get(),second);
    PCUITEMID_CHILD children[]={one.get(),two.get()};
    ComPtr<IContextMenu> context;
    check(parent->GetUIObjectOf(hwnd,2,children,IID_IContextMenu,nullptr,reinterpret_cast<void**>(context.GetAddressOf())),"get two-item context menu");
    HMENU menu=CreatePopupMenu();
    require(menu!=nullptr,"cannot create popup menu");
    const HRESULT count=context->QueryContextMenu(menu,0,1,0x6fff,CMF_NORMAL);
    check(count,"populate registered context menu");
    int command=-1;
    for(UINT offset=0;offset<static_cast<UINT>(HRESULT_CODE(count));++offset) {
        wchar_t name[256]{},label[256]{};
        context->GetCommandString(offset,GCS_VERBW,nullptr,reinterpret_cast<char*>(name),static_cast<UINT>(std::size(name)));
        GetMenuStringW(menu,1+offset,label,static_cast<int>(std::size(label)),MF_BYCOMMAND);
        std::wstring display(label);
        display.erase(std::remove(display.begin(),display.end(),L'&'),display.end());
        if(std::wstring(name)==verb || display==L"用 qing-zip 压缩…"){
            require(command<0,"ambiguous file compression command");command=static_cast<int>(offset);
        }
    }
    require(command>=0,"registered verb was not offered for the multi-selection");
    CMINVOKECOMMANDINFOEX invoke{};invoke.cbSize=sizeof(invoke);invoke.hwnd=hwnd;
    invoke.fMask=CMIC_MASK_UNICODE|CMIC_MASK_NOASYNC|CMIC_MASK_FLAG_NO_UI;
    invoke.lpVerb=MAKEINTRESOURCEA(command);invoke.lpVerbW=MAKEINTRESOURCEW(command);invoke.nShow=SW_SHOWNORMAL;
    check(context->InvokeCommand(reinterpret_cast<LPCMINVOKECOMMANDINFO>(&invoke)),"invoke actual menu item");
    DestroyMenu(menu);
}
void exercise(const fs::path& exe,const fs::path& first,const fs::path& second) {
    require(first.parent_path()==second.parent_path(),"select siblings");
    WNDCLASSW wc{};wc.hInstance=GetModuleHandleW(nullptr);wc.lpszClassName=sinkClass;wc.lpfnWndProc=sink;
    require(RegisterClassW(&wc)!=0,"cannot register fixture window class");
    require(!FindWindowW(sinkClass,sinkTitle),"a real qing-zip instance already exists");
    HANDLE mutex=CreateMutexW(nullptr,TRUE,sinkMutex);
    require(mutex!=nullptr && GetLastError()!=ERROR_ALREADY_EXISTS,"a real qing-zip mutex already exists");
    HWND hwnd=CreateWindowW(sinkClass,sinkTitle,WS_OVERLAPPED,0,0,1,1,nullptr,nullptr,wc.hInstance,nullptr);
    require(hwnd!=nullptr,"cannot create forwarding receiver");

    invokeMenu(hwnd,first,second);
    const auto deadline=std::chrono::steady_clock::now()+40s;
    while(received.empty() && std::chrono::steady_clock::now()<deadline) pumpMessages(std::chrono::steady_clock::now()+100ms);
    require(!received.empty(),"no qing-zip single-instance forwarding");
    pumpMessages(std::chrono::steady_clock::now()+5s);
    require(received.size()==1,"more than one single-instance group");
    const auto args=arguments(received[0]);
    require(args.size()==5,"expected cwd, executable, switch, and two paths");
    require(fs::equivalent(args[1],exe) && args[2]==L"--compress-files","unexpected executable or mode");
    require((fs::equivalent(args[3],first)&&fs::equivalent(args[4],second)) ||
            (fs::equivalent(args[3],second)&&fs::equivalent(args[4],first)),"selection paths changed");
    std::cout << "{\"selection_items\":2,\"advertised_verb\":true,\"groups_received\":1,\"matched_paths\":2,\"settle_seconds\":5}\n";
    DestroyWindow(hwnd);ReleaseMutex(mutex);CloseHandle(mutex);
}
void invokeForGui(const fs::path& exe, const fs::path& first, const fs::path& second, DWORD expectedPid) {
    require(first.parent_path()==second.parent_path(),"select siblings");
    HWND primary=FindWindowW(sinkClass,sinkTitle);
    require(primary!=nullptr,"no running GUI single-instance window");
    DWORD pid=0;
    GetWindowThreadProcessId(primary,&pid);
    require(pid==expectedPid,"single-instance window is not owned by the launched GUI");
    HANDLE process=OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,FALSE,pid);
    require(process!=nullptr,"cannot identify primary GUI");
    wchar_t path[32768]{}; DWORD length=static_cast<DWORD>(std::size(path));
    const BOOL found=QueryFullProcessImageNameW(process,0,path,&length);
    CloseHandle(process);
    require(found && fs::equivalent(path,exe),"unexpected primary executable");
    invokeMenu(nullptr,first,second);
    std::cout<<"{\"real_primary_pid\":"<<pid<<",\"invoked_for_files\":2}\n";
}
} // namespace
int wmain(int argc,wchar_t** argv) {
    try {
        require((argc==4||argc==5)&&envIs(L"GITHUB_ACTIONS",L"true")&&envIs(L"RUNNER_ENVIRONMENT",L"github-hosted"),"requires isolated hosted Windows runner");
        check(OleInitialize(nullptr),"initialize shell apartment");
        if(argc==4) exercise(argv[1],argv[2],argv[3]);
        else invokeForGui(argv[1],argv[2],argv[3],std::stoul(argv[4]));
        OleUninitialize();return 0;
    } catch(const std::exception& e) { std::cerr<<e.what()<<'\n';return 1; }
}
