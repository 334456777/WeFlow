## 下载

不知道选哪个？先看你的系统，再往下看对应的说明。**绝大多数 Windows 电脑选 x64 就对了。**

### Windows

#### [weflow-{{TAG}}-windows-x64.zip](https://github.com/{{REPO}}/releases/download/{{TAG}}/weflow-{{TAG}}-windows-x64.zip)
适合**绝大多数 Windows 电脑**，包括常见的台式机和笔记本（Intel 或 AMD 处理器）。不确定就选这个。

#### [weflow-{{TAG}}-windows-arm64.zip](https://github.com/{{REPO}}/releases/download/{{TAG}}/weflow-{{TAG}}-windows-arm64.zip)
只适合**使用 ARM 处理器的 Windows 电脑**，比如搭载骁龙芯片的笔记本、Surface Pro X。这类电脑比较少见。

> 怎么确认：打开「设置 → 系统 → 系统信息」，看“系统类型”。写着“x64 处理器”选上面那个，写着“ARM 处理器”选这个。

### macOS（苹果电脑）

#### [weflow-{{TAG}}-macos-arm64.tar.gz](https://github.com/{{REPO}}/releases/download/{{TAG}}/weflow-{{TAG}}-macos-arm64.tar.gz)
适合**2020 年底之后发售的 Mac**，也就是用苹果自家芯片（M1、M2、M3 等）的机器。

#### [weflow-{{TAG}}-macos-x64.tar.gz](https://github.com/{{REPO}}/releases/download/{{TAG}}/weflow-{{TAG}}-macos-x64.tar.gz)
适合**较老的 Mac**，也就是用 Intel 处理器的机器。

> 怎么确认：点屏幕左上角苹果图标 →「关于本机」。“芯片”一栏写着 Apple M1 之类的，选第一个；写着 Intel 的，选第二个。

### Linux

#### [weflow-{{TAG}}-linux-x64.tar.gz](https://github.com/{{REPO}}/releases/download/{{TAG}}/weflow-{{TAG}}-linux-x64.tar.gz)
适合**常见的 Linux 电脑和服务器**（Intel 或 AMD 处理器）。

#### [weflow-{{TAG}}-linux-arm64.tar.gz](https://github.com/{{REPO}}/releases/download/{{TAG}}/weflow-{{TAG}}-linux-arm64.tar.gz)
适合**使用 ARM 处理器的 Linux 设备**，比如运行 64 位系统的树莓派、部分云服务器。

> 怎么确认：打开终端输入 `uname -m`。显示 `x86_64` 选 x64，显示 `aarch64` 选 arm64。

### 下载后怎么用

每个压缩包里有 `weflow` 程序、`README.md` 和 `LICENSE`。下载后解压（tar.gz文件用`tar -zxvf <文件名.tar.gz>`）就能使用里面的 `weflow`。

> 没见过？不会用？把文件扔给蓝色大肥鱼，她会解释
