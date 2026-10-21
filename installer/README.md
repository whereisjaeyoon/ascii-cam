# ASCII Cam graphical installer (Rust)

기존 UW–Madison 스타일 컨트롤러와 같은 Badger Red `#C5050C`, 오프화이트 `#F8F7F4`, 카드/버튼 디자인을 사용하는 eframe 설치 프로그램입니다.

## 설치하는 것

- **클라이언트**: 기존 `controller-ui`의 Release EXE → `ascii-cam-ui.exe`
- **가상 카메라 호스트**: 기존 C++ 콘솔 호스트 → `ascii-cam-host.exe`
- **카메라 소스 DLL**: `ascii-cam.dll`
- **필요한 런타임**: Visual Studio의 배포용 x64 CRT DLL, 바이너리가 실제 참조한다면 OpenCV DLL
- **제거 프로그램**: 설치 프로그램 자체 `ascii-cam-setup.exe`

배포 파일 `ASCII-Cam-Setup.exe` **하나에 위 파일들을 내장**합니다. 설치 대상 PC에 Rust/Cargo/Visual Studio를 설치할 필요가 없고, 설치 중 네트워크 다운로드도 하지 않습니다. Windows 11 x64 (build 22000 이상), 그래픽 드라이버, 실제 웹캠 및 Windows 카메라 접근 권한은 필요합니다. 설치 프로그램의 CRT는 정적 링크합니다.

클라이언트의 장치 목록 기능은 유지됩니다. 기존 ON/OFF 및 필터 버튼은 여전히 UI 상태만 바꾸며 카메라 호스트와 연결되지 않습니다. 이번 작업은 **설치 프로그램**입니다. 시작 메뉴의 카메라 호스트 바로가지를 별도로 실행하세요.

## 프로젝트에 적용

압축 안의 `installer` 폴더를 프로젝트의 `installer` 폴더로 적용합니다. 기존 파일 백업을 포함하는 스크립트:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File "$env:USERPROFILE\Downloads\ascii-cam-installer-ui\installer\Apply-Installer.ps1"
```

기본 저장소 위치는 `C:\Users\jkl27\ascii-cam`입니다. 다른 위치라면 `-RepoPath`를 전달하세요.

## 현재 개발 PC에서 빌드하기

이미 영상 출력에 성공한 DLL과 기존 호스트를 사용하는 방법입니다. 빌드 스크립트는 클라이언트와 설치 프로그램을 Release x64로 컴파일합니다.

```powershell
cd C:\Users\jkl27\ascii-cam\installer
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-installer.ps1 -SkipNativeBuild -CameraHost "C:\Users\jkl27\source\repos\ascii-cam\ascii-cam\ascii-cam\x64\Release\ascii-cam.exe"
```

DLL 기본 경로:
`C:\Users\jkl27\ascii-cam\ascii-cam-source\x64\Release\ascii-cam.dll`

DLL 경로도 바꾸려면 `-CameraDll "실제 DLL 경로"`를 추가하세요.

기본적으로 Visual Studio 설치 경로에서 x64 재배포용 CRT와 dumpbin을 찾습니다. 찾지 못하면 Developer PowerShell for Visual Studio에서 실행하거나 `-RuntimeDirectory`로 VC\Redist\MSVC\버전\x64\Microsoft.VC*.CRT 폴더를 지정하세요. OpenCV가 필요한 경우 기본 `C:\opencv\build\x64\vc16\bin` 또는 `-OpenCvBin`을 사용합니다.

DLL과 호스트까지 현재 저장소에서 빌드하려면:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\build-installer.ps1
```

이 경우 두 C++ 솔루션에 필요한 NuGet 복원 및 OpenCV 경로 설정을 먼저 완료해야 합니다. 스크립트는 프로젝트 설정을 수정하지 않습니다.

출력:

```text
C:\Users\jkl27\ascii-cam\installer\dist\ASCII-Cam-Setup.exe
```

**이 EXE 하나만 배포**하세요. Cargo.lock을 포함하며 --locked로 빌드합니다. 패키징은 x64 PE, 종속 DLL, COM exports 4개와 실제 EXE의 requireAdministrator 매니페스트를 검사합니다. 원본 DLL을 실행해서 검증하거나 regsvr32를 호출하지 않습니다.

## 설치 화면 / 사용

1. 카메라 앱, Zoom/OBS, 클라이언트, 기존 카메라 호스트를 닫습니다.
2. `ASCII-Cam-Setup.exe`를 실행하고 UAC 권한 요청을 허용합니다.
3. `Install ASCII Cam`을 누릅니다. 이미 설치됐다면 Update로 표시됩니다.
4. 기본 위치 `C:\Program Files\AsciiCam`에 설치합니다. 시작 메뉴 바로가지를 만들고, 선택하면 공용 바탕 화면에도 바로가지를 만듭니다.
5. 완료 후 시작 메뉴에서 **ASCII Cam Controller** 또는 **ASCII Cam Camera**를 실행합니다. 설치 프로그램이 관리자 권한 클라이언트를 자동으로 실행하지는 않습니다.

DLL이 계속 잠겨 있으면 관리자 cmd에서 `net stop FrameServer` 후 재시도하고, 설치 성공/실패와 관계없이 `net start FrameServer`로 다시 켜세요. 그래도 잠겨 있으면 재부팅 후 카메라 앱을 열기 전에 설치합니다. 설치 프로그램은 다른 앱을 강제 종료하거나 시스템 서비스를 자동 중지하지 않습니다.

## Rust에서 하는 Windows 등록

64비트 HKLM 레지스트리에 직접 기록합니다.

- `SOFTWARE\Classes\CLSID\{3CAD447D-F283-4AF4-A3B2-6F5363309F52}\InprocServer32`
  - 기본값(REG_SZ): 설치된 `ascii-cam.dll` 절대 경로
  - ThreadingModel(REG_SZ): `Both`
- `SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\AsciiCam`
  - DisplayName, DisplayVersion, InstallLocation, DisplayIcon, EstimatedSize
  - UninstallString: `"C:\Program Files\AsciiCam\ascii-cam-setup.exe" --uninstall`
  - NoModify / NoRepair(REG_DWORD)

등록 방식은 이 프로젝트 DLL의 DllRegisterServer 구현과 동일합니다. 다른 COM DLL에 필요한 별도 등록 절차까지 자동으로 지원하는 일반 DLL 설치기는 아닙니다.

## 제거

Windows 설정 → 앱 → 설치된 앱 → **ASCII Cam → 제거**를 선택하거나, 설치 프로그램의 **Uninstall** 페이지에서 `Uninstall ASCII Cam`을 누릅니다.

- 설치 매니페스트에 기록된 앱 파일과 런타임, 소유한 바로가지를 제거합니다.
- 해당 DLL 경로를 여전히 가리키는 COM 등록만 해제합니다. 다른 설치가 가져간 등록은 보존합니다.
- 자신이 관리한 레지스트리 값만 삭제하며 다른 값/하위 키는 보존합니다.
- 프로젝트 소스와 설치 폴더의 무관한 파일은 지우지 않습니다. 폴더 전체를 재귀 삭제하지 않습니다.
- 이전 DLL-only v0.1 설치도 제거할 수 있습니다.
- 실행 중인 제거 EXE는 Windows가 즉시 삭제하지 못할 수 있습니다. 고유한 임시 이름으로 바꾼 뒤 재부팅 시 그 파일만 삭제 예약하거나, 실패하면 완료 화면에 수동 삭제할 파일을 표시합니다. 안정된 설치 파일 경로 자체를 재부팅 삭제 예약하지 않으므로 재설치한 새 EXE가 지워지지 않습니다.

## 실패 / 업데이트 처리

- 설치·제거는 별도 작업 스레드에서 수행하고, 진행 중에는 창 닫기를 막습니다.
- 머신 단위 mutex로 동시 설치·제거를 막습니다.
- 파일은 임시 파일을 완전히 쓴 뒤 교체하므로 잠긴 DLL을 잘라 쓰지 않습니다.
- 실패하면 이번 시도의 파일과 레지스트리 값을 복원합니다. 복원 실패도 GUI에 표시합니다.
- 정상 반환/실패에 대한 복원이며, 전원 중단/프로세스 강제 종료에 대한 영속 트랜잭션은 아닙니다.
- 수동 변경되거나 잘못된 설치 매니페스트, 경로 탈출, 심볼릭 링크/재분석 지점은 거부합니다.

## 테스트 / 한계

```powershell
cargo test --locked
cargo check --locked --target x86_64-pc-windows-msvc
```

`cargo run`은 페이로드 없는 GUI 미리보기입니다. Install은 비활성화됩니다. 배포본은 반드시 `build-installer.ps1`로 빌드합니다.

이 환경에서 Windows 대상 Rust 컴파일 검사 및 파일 복원/PE/경로 검사 테스트를 실행했습니다. 실제 Windows의 파일 잠금, COM 등록, 바로가기, UAC, 제거/재설치는 Windows에서 추가 검증해야 합니다. 이전 DLL-only 설치기와 이번 GUI 설치기는 별도 버전입니다.

이 코드에는 제품 코드 서명이 포함되지 않습니다. UI 그리기는 기존 클라이언트와 같은 OpenGL GPU 렌더러를 사용하며 파일 복사·레지스트리·바로가기 작업은 CPU/Windows API 작업입니다.

공식 근거:
- https://learn.microsoft.com/en-us/windows/win32/api/mfvirtualcamera/nf-mfvirtualcamera-mfcreatevirtualcamera
- https://learn.microsoft.com/en-us/windows/win32/com/inprocserver32
- https://learn.microsoft.com/en-us/windows/win32/secbp/running-with-administrator-privileges
- https://learn.microsoft.com/en-us/powershell/scripting/samples/working-with-software-installations
