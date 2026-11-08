#define UNICODE
#include <windows.h>
#include <shlobj.h>
#include <shobjidl.h>
#include <wrl/client.h>
#include <cstdio>
using Microsoft::WRL::ComPtr;
static PCWSTR source;
static void sample(IDataObject* data,IDataObjectAsyncCapability* async) {
 BOOL running=0;HRESULT value=async->InOperation(&running);
 printf("active=%d(%08lx)",running,value);
 for(auto name:{CFSTR_PERFORMEDDROPEFFECT,CFSTR_LOGICALPERFORMEDDROPEFFECT,CFSTR_PASTESUCCEEDED}) {
  FORMATETC format={(CLIPFORMAT)RegisterClipboardFormatW(name),nullptr,DVASPECT_CONTENT,-1,TYMED_HGLOBAL};
  STGMEDIUM medium{};auto hr=data->QueryGetData(&format);printf(" %08lx",hr);
  if(hr==S_OK && SUCCEEDED(data->GetData(&format,&medium))) {
   auto p=(DWORD*)GlobalLock(medium.hGlobal);if(p){printf("=%lu",*p);GlobalUnlock(medium.hGlobal);}ReleaseStgMedium(&medium);
  }
 }
 puts("");
}
static HRESULT perform(HWND hwnd) {
 PIDLIST_ABSOLUTE id=nullptr; HRESULT hr=SHParseDisplayName(source,nullptr,&id,0,nullptr); if(FAILED(hr))return hr;
 PCIDLIST_ABSOLUTE pidl=id;
 ComPtr<IShellItemArray> items;hr=SHCreateShellItemArrayFromIDLists(1,&pidl,&items);CoTaskMemFree(id);if(FAILED(hr))return hr;
 ComPtr<IDataObject> data;hr=items->BindToHandler(nullptr,BHID_DataObject,IID_PPV_ARGS(&data));if(FAILED(hr))return hr;
 ComPtr<IDataObjectAsyncCapability> async;hr=data.As(&async);if(FAILED(hr))return hr;
 hr=async->SetAsyncMode(TRUE);if(FAILED(hr))return hr;
 printf("before ");sample(data.Get(),async.Get());
 DWORD effect=0;hr=SHDoDragDrop(hwnd,data.Get(),nullptr,DROPEFFECT_COPY,&effect);
 printf("returned %08lx effect=%lu ",hr,effect);sample(data.Get(),async.Get());
 for(int i=0;i<20;++i){
  MSG msg;while(PeekMessageW(&msg,nullptr,0,0,PM_REMOVE)){TranslateMessage(&msg);DispatchMessageW(&msg);}
  Sleep(75);printf("tick%d ",i);sample(data.Get(),async.Get());
 }
 return hr;
}
static LRESULT CALLBACK window(HWND h,UINT m,WPARAM w,LPARAM l) {
 if(m==WM_LBUTTONDOWN){auto result=perform(h);PostQuitMessage(FAILED(result)?1:0);return 0;}
 return DefWindowProcW(h,m,w,l);
}
int wmain(int argc,wchar_t** argv){
 if(argc!=2)return 2;setbuf(stdout,nullptr);source=argv[1];if(FAILED(OleInitialize(nullptr)))return 3;
 WNDCLASSW wc{};wc.lpfnWndProc=window;wc.hInstance=GetModuleHandleW(nullptr);wc.lpszClassName=L"QingSignalFixture";RegisterClassW(&wc);
 HWND h=CreateWindowExW(WS_EX_TOPMOST,wc.lpszClassName,L"Qing signal fixture",WS_OVERLAPPEDWINDOW|WS_VISIBLE,0,0,260,240,nullptr,nullptr,wc.hInstance,nullptr);
 if(!h)return 4;MSG msg;while(GetMessageW(&msg,nullptr,0,0)>0){TranslateMessage(&msg);DispatchMessageW(&msg);}DestroyWindow(h);OleUninitialize();return (int)msg.wParam;
}
