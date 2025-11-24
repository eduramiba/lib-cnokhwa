windows-all: win-x64 win-x86

win-x64:
	cargo build --release --target x86_64-pc-windows-msvc

win-x86:
	cargo build --release --target=i686-pc-windows-msvc

windows7-all: win7-x64 win7-x86 # We want to make sure our library works on Windows 7 too

win7-x64:
	cargo +nightly build -Z build-std=std,panic_abort --target x86_64-win7-windows-msvc --release

win7-x86:
	cargo +nightly build -Z build-std=std,panic_abort --target i686-win7-windows-msvc --release

linux-all: linux-x64 linux-x86

linux-x64:
	cargo build --release --target=x86_64-unknown-linux-gnu

linux-x86:
	cargo build --release --target=i686-unknown-linux-gnu