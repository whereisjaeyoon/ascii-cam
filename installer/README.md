# ASCII Cam Rust installer

기존 **C++ 카메라용 `ascii-cam.dll`**을 설치하는 Rust 프로그램입니다.
`hello_append.dll`은 일반 DLL이며 COM 서버가 아니므로 등록 대상이 아닙니다.
이 설치 프로그램은 append DLL을 COM DLL로 변환하지 않습니다.

## 하는 일

1. 입력 DLL이 x64 DLL이고 필요한 COM 공개 함수 4개가 있는지 검사합니다.
2. 관리자 권한이 없으면 UAC 승인 창을 띄웁니다.
3. DLL을 `C:\Program Files\AsciiCam\ascii-cam.dll`에 복사합니다.
4. DLL의 기존 DllRegisterServer 코드와 같은 CLSID/ThreadingModel을 Rust로 직접 기록합니다.
5. 제거용 설치 프로그램을 같은 폴더에 복사합니다.
6. Windows의 **설정 → 앱 → 설치된 앱**에 `ASCII Cam`을 등록합니다.

UI, 카메라 실행 프로그램, 필터 코드는 변경하지 않습니다. 카메라를 자동 실행하지 않습니다.
설치된 앱 항목은 MSI 설치가 아닌 이 Rust 설치 프로그램의 제거 명령을 가리킵니다.

## 폴더와 빌드

이 `installer` 폴더를 아래 위치에 놓습니다.

```text
C:\Users\jkl27\ascii-cam\installer
```

PowerShell에서 실행합니다. Rust와 Visual Studio C++ 빌드 도구가 필요합니다.

```powershell
cd C:\Users\jkl27\ascii-cam\installer
cargo build --release --target x86_64-pc-windows-msvc
```

결과물:

```text
target\x86_64-pc-windows-msvc\release\ascii-cam-setup.exe
```

## 설치

카메라 앱, Zoom, OBS, 실행 중인 `ascii-cam.exe`를 닫은 뒤 실행합니다.
입력 DLL은 앞서 exports 4개를 확인한 Release x64 결과물이어야 합니다.

```powershell
.\target\x86_64-pc-windows-msvc\release\ascii-cam-setup.exe install "C:\Users\jkl27\ascii-cam\ascii-cam-source\x64\Release\ascii-cam.dll"
```

UAC 창에서 예를 선택합니다. 새로 열린 관리자 창의 **Installation complete**가
실제 완료 결과입니다. 부모 창의 'Opened the administrator installer window'는
관리자 창을 열었다는 뜻이며 아직 설치 성공을 의미하지 않습니다.

DLL이 사용 중이면 덮어쓰기를 실패 처리합니다. 이 경우 카메라 관련 앱과 호스트를
닫고 다시 시도합니다. 프레임 서버가 DLL을 계속 사용하면 재부팅 후 설치합니다.
설치 프로그램은 서비스나 다른 앱을 강제 종료하지 않습니다.

## 등록 확인 (읽기 전용)

```powershell
& "C:\Program Files\AsciiCam\ascii-cam-setup.exe" status
```

`Registered DLL`, `ThreadingModel: Both`, `File exists: true`,
`Installed-app entry exists: true`가 표시되는지 확인합니다.
실제 카메라의 영상 출력 여부는 별도 실행 검증이 필요합니다.

## 레지스트리 항목

64비트 HKLM 뷰에 기록합니다.

### 카메라 COM 등록

```text
HKEY_LOCAL_MACHINE\SOFTWARE\Classes\CLSID\{3CAD447D-F283-4AF4-A3B2-6F5363309F52}\InprocServer32
```

| 이름 | 종류 | 값 |
|---|---|---|
| (기본값) | REG_SZ | C:\Program Files\AsciiCam\ascii-cam.dll |
| ThreadingModel | REG_SZ | Both |

### 설치된 앱 / 제거 항목

```text
HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\AsciiCam
```

| 이름 | 종류 | 값 |
|---|---|---|
| DisplayName | REG_SZ | ASCII Cam |
| DisplayVersion | REG_SZ | 0.1.0 |
| InstallLocation | REG_SZ | C:\Program Files\AsciiCam |
| UninstallString | REG_SZ | "C:\Program Files\AsciiCam\ascii-cam-setup.exe" uninstall |
| NoModify | REG_DWORD | 1 |
| NoRepair | REG_DWORD | 1 |

Program Files가 다른 드라이브에 있으면 시스템 설정에 맞춰 경로가 바뀝니다.
`regedit`나 `regsvr32`를 수동으로 실행할 필요는 없습니다.

## 제거

Windows 설정의 설치된 앱에서 `ASCII Cam`을 제거하거나 다음 명령을 실행합니다.

```powershell
& "C:\Program Files\AsciiCam\ascii-cam-setup.exe" uninstall
```

설치 경로를 검증한 뒤 설치한 DLL과 앱 목록 항목을 제거합니다. COM 경로가 이 설치본을
가리킬 때만 해당 CLSID를 제거합니다. 프로젝트 소스와 UI는 지우지 않습니다.
실행 중인 제거 EXE는 Windows 재시작 때 삭제하도록 예약합니다. 예약이 실패하면
창을 닫은 뒤 해당 파일을 수동 삭제하라는 메시지를 표시합니다.
폴더는 비어 있을 때만 삭제합니다.

## 오류 및 검증 범위

- 호환되지 않는 DLL, 없는 파일, x86 DLL, COM 함수가 없는 DLL은 거부합니다.
- 파일은 임시 파일 작성 후 교체합니다. 파일/레지스트리 작업 실패 시 이전 값을
  복구하도록 시도하며 복구 실패도 출력합니다. 강제 종료나 정전까지 보장하는
  트랜잭션 또는 MSI 패키지는 아닙니다.
- COM 공개 함수의 존재를 검사할 뿐, 임의 DLL의 CLSID를 자동 추출하지 않습니다.
  이 저장소에서 빌드한 카메라 DLL 전용입니다.
- x64 Windows 대상 `cargo check`와 DLL 검사 단위 테스트를 수행했습니다.
  실제 UAC 승인, HKLM 쓰기, 설치된 앱 표시 및 제거는 Windows에서 확인해야 합니다.
