# 微信聊天记录解密

原文：https://sarv.blog/posts/chatlog/

2025-03-25 · 6 分钟 · Sarv

### 概述

为了整理历史聊天记录，翻阅了不少网络资料和代码库，很多项目的目标都是如何 1:1 还原微信聊天界面，这和我的需求有些出入，我更希望能够有：

- Local First 且查询自由，支持任意时段任意对象的全文检索。
- 能够非常简单的和大模型工具配合，帮助我从聊天记录中获取相关信息。  
  既然没有合适的工具，那就考虑自己实现一个，于是就有了 chatlog 这个项目。

项目地址：<https://github.com/sjzar/chatlog>

### 微信本地数据库

微信的聊天数据都保存在本地的数据库文件中，这就是我们要聊的第一部分——微信本地数据库。  
微信本地数据库文件是一组加密后的 SQLCipher 文件，由腾讯的 [WCDB 项目](https://github.com/Tencent/wcdb) 支持。  
SQLCipher 的机制是以增加 10-15% 的开销，100% 加密本地的 SQLite 文件，并提供与 SQLite 相同的 API。  
首次在设备登录微信时，会计算出一组密钥用于加密本地的数据库文件，这组密钥在每个账户、每台设备上都是不一样的。  
在同账号同设备上更换密钥的频率不高，更换密钥意味着需要对全部数据重新加密，重度使用的微信账号数据量不小，所以一般是微信大版本更新时更换密钥。  
虽然微信在各个平台不同大版本中均使用 WCDB 项目，但是加密方式却有所区别。下面是总结的信息：

| 平台 | 版本 | 加密方式 | 加密参数 | 备注 |
| --- | --- | --- | --- | --- |
| Windows | 3.x | SQLCipher v3 | Page Size 4096 / KDF 迭代 64000 / HMAC 算法 SHA-1 / KDF 算法 SHA-1 |  |
| Windows | 4.0 | SQLCipher v4 | Page Size 4096 / KDF 迭代 256000 / HMAC 算法 SHA-256 / KDF 算法 SHA-256 | SQLCipher v4 默认配置 |
| macOS | 3.x | SQLCipher v3 | Page Size 1024 / HMAC 算法 SHA-1 | 比较特殊，直接使用原始密钥 |
| macOS | 4.0 | SQLCipher v4 | Page Size 4096 / KDF 迭代 256000 / HMAC 算法 SHA-256 / KDF 算法 SHA-256 | 与 Windows 4.x 一致 |

了解完数据库加密方式后，简单聊下数据结构。  
不同版本的数据结构差异较大，例如 Windows 3.x 版本的数据库，聊天记录是保存在多个 db 文件中，并按时间分库，最新的聊天记录都在同一个文件里，在同一个 db 文件中，所有聊天记录都在一张大表里；macOS 3.x 版本的数据库，聊天记录是按照聊天对象分表的，每个聊天对象有单独的表，再将这些表分配到多个 db 文件中，这意味着如果部分聊天对象记录较多，那么对应的 db 文件就比较大；4.0 版本的微信，数据结构又发生了变化，结合了上述两个方案的特点，先按照时间划分 db 文件，再为每个聊天对象设置单独的表，并且这回 Windows 和 macOS 采用的是相同的方案。

4.0 版本的数据结构其实是更像 Windows 3.x 版本，这也愈发让人觉得 macOS 3.x 版本的数据结构特别奇怪，甚至不像是同一个团队出的产品。  
例如在其他版本中，群聊叫做 ChatRoom，而在 macOS 3.x 中叫做 Group；在其他版本中，群成员的信息是保存在 Contact 表中的，而在 macOS 3.x 版本中群成员是独立维护的。（吐槽的原因是由于 chatlog 项目希望支持所有主流版本的微信，所以为了 macOS 3.x 做了不少兼容工作）

### 本地数据库解密

目前网上的解密方式，一般是从微信进程中获取密钥，再通过密钥解密数据库后查询数据，这也是为什么 Windows 下工具较多，而 macOS 下没有特别方便的工具，因为 Windows 下获取微信内存数据比较简单，可以通过 Windows API 直接读取内存数据，而在 macOS 下有安全机制限制访问其他进程的内存数据，需要先关闭 SIP 才能操作。

获取密钥有几种方式，一种是通过内存基址计算偏移来获取，获取密钥速度快，缺点就是需要手动维护每个小版本的内存基址，代表项目是 [xaoyaoo/PyWxDump](https://github.com/xaoyaoo/PyWxDump/blob/master/pywxdump/WX_OFFS.json) 。  
另一种方式就是通过内存数据暴力搜索，微信在运行时会打开数据库文件句柄，那么通常在内存数据中会有密钥信息，完整扫描整块内存区域不太可行，但是通过一些数据特征缩小扫描范围，就让这个方案变得可行了，代表项目是 [0xlane/wechat-dump-rs](https://github.com/0xlane/wechat-dump-rs)。  
第三种方案是 Hook 方案，向微信进程注入动态库后直接调用相关函数获取密钥数据，通常使用 Hook 方案的项目重点都不在获取密钥，而是对微信本身的功能做魔改，例如防撤回、找单删好友、抢红包等，这个方案也是封号风险最大的。

chatlog 项目采用的是方案二，也就是通过读取微信内存，暴力搜索查找数据密钥。  
经过测试，Windows 系统下一般 20~30ms 可以找到密钥，macOS 系统下需要约 20 秒的时间，其中大部分时间是在用于 dump 内存数据。  
这个方案的封号风险我认为不大，项目并不会操作微信的 API 做额外的请求，获取密钥后的所有操作都和微信进程不相关了。

### macOS 版本的解密方案

Windows 平台下有不少成熟的解密项目，但是 macOS 这边相关项目较少，这也给我研究解密带来不少困难。  
网上流传最广的 [macOS 微信解密方案](https://github.com/xaoyaoo/PyWxDump/blob/master/doc/MAC%E6%95%B0%E6%8D%AE%E5%BA%93%E8%A7%A3%E5%AF%86.md)，是利用 lldb 的断点功能，在设置 sqlite3\_key 的地方下断，然后登录微信触发断点后，从寄存器中获取密钥。  
这个方案只在 3.8.5 以下版本的微信中可用，更高版本的微信就无法通过这个方案获取密钥了。  
我的思路还是先把内存 dump 出来，然后分析内容，既然选择内存暴力破解，那么只要选好特征，避免全量扫描即可。  
本地有加密的数据库文件和 dump 出来的内存二进制文件，反复测试后获得密钥数据，再通过密钥数据归纳特征，就有了现在的解密方案。

### 数据处理

有了密钥后就需要对数据库文件进行处理，刚开始想的是利用密钥对数据库文件做一个即时解密的转换层，这样不需要额外保存一份数据文件。但是由于对 sqlite 不太熟悉，这个方案难度较大，索性还是先将数据解密成标准的 sqlite 文件再进行读取。  
由于不同版本的微信数据库结构还有差异，还简单做了 wrap 处理，多媒体数据还没来得及处理，不过这个难度不大，总会处理完的。  
关于多媒体数据，我的想法是仍然放在微信数据目录内，通过 chatlog 的 HTTP 服务做文件代理。例如聊天记录中引用某个文件，就转换为 chatlog 提供的 localhost HTTP 链接，访问链接就读取到了微信数据目录内的相关文件。优点是无需额外保存一份多媒体数据，缺点就是部分数据可能需要即时转码。不过方案肯定还能优化，后续再聊了。

### MCP

好的，到了最关键的时刻，辛辛苦苦把聊天记录弄出来，就是要把自己的数据用起来。  
基于 MCP 的 SSE 方案写了服务，目前能够支持 ChatWise、Claude Desktop、Monica Code、Cline 等工具，但是只有 ChatWise 是原生支持 SSE，其他方案都是通过 mcp-proxy 转发。  
先看下效果：

[![](assets/cdn.sarv.blog/IMG-20250325003314415.png) 

ChatWise](assets/cdn.sarv.blog/IMG-20250325003314415.png)
[![](assets/cdn.sarv.blog/IMG-20250325003352392.png) 

Claude Desktop](assets/cdn.sarv.blog/IMG-20250325003352392.png)
[![](assets/cdn.sarv.blog/IMG-20250325003612045.png) 

Monica Code](assets/cdn.sarv.blog/IMG-20250325003612045.png)

MCP 服务支持 stdio 和 SSE 两种传输方式，但是由于项目不完善，只有 SPEC 没落地，连官方的 Claude Desktop 都不支持 SSE 服务器。  
stdio 方案是由本地聊天工具起子进程，通过 stdin 和 stdout 和子进程传输数据。  
如果有外部服务想作为 MCP，在现在的 Claude Desktop 上只能想办法弄个子进程服务，把相关请求代理出来。  
SSE 方案，是基于 HTTP 协议做的，用长+短两个连接来连 Client 和 Server。  
长连接就是 SSE，Client 发给 Server，Server 返回一个 sessionid，然后连接保持住不断开，后续服务端向客户端发数据都用这个 SSE 连接。  
客户端向服务端请求就再发起一个 HTTP 请求给服务端，叫做 /messages，然后服务端给这个 /messages 请求返回一个202，再用 SSE 连接返回需要的数据，这样实现双向通信。  
但是这个方案也有坑，就是没约定 SSE 连接多久发心跳，断了咋重连，所以在 SSE 断开后，客户端再发 /messages，服务端就无法响应了。  
好消息是官方也注意到这个问题了，支持了新的流式 HTTP 模式：[[RFC] Replace HTTP+SSE with new “Streamable HTTP” transport by jspahrsummers · Pull Request #206 · modelcontextprotocol/specification · GitHub](https://github.com/modelcontextprotocol/specification/pull/206)。  
还想吐槽的是，官方将服务器提供的内容归类为 Resource、Prompt、Tool，但是目前官方的 Claude Desktop 还不支持 Resource。  
本来想将 chatlog 作为 Resource 的，写完代码发现没法用，只能作为 Tool 调用，希望接下来尽快完善～

### Terminal UI

临近结尾，回过头聊一下项目的 TUI 吧。  
这次用的是 [rivo/tview](https://github.com/rivo/tview) 项目，第一次使用这个项目，效果看起来还不错。  
想到前两年做的命令行项目，还是读取 readline 后自己维护页面，现在的 TUI 效果真是好太多了。

### 总结

以上就是本次项目的一些碎碎念，总结下来就是写了个微信聊天记录解密项目，希望这个工具能够帮到大家。
