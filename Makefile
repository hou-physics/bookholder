.PHONY: setup build test dev install clean

test:
	cargo test

dev:
	cd app && ui/node_modules/.bin/tauri dev

build:
	cd app/ui && npm install
	cd app && ui/node_modules/.bin/tauri build

install: build
	# 先杀掉旧进程再替换再启动——否则 macOS 上 `open` 只会把仍在跑的旧进程
	# 拉到前台，新构建的二进制永远不会真正运行（实测踩过这个坑）。
	-pkill -f "Bookholder.app/Contents/MacOS/bookholder-app"
	sleep 1
	rm -rf /Applications/Bookholder.app
	cp -R target/release/bundle/macos/Bookholder.app /Applications/
	open /Applications/Bookholder.app

setup: test install
	@echo "✅ Bookholder 已安装并启动。开机自启请在应用设置页勾选。"

clean:
	cargo clean && rm -rf app/ui/node_modules app/ui/dist
