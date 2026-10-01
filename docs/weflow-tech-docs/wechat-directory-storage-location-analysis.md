---
id: wechat-directory-storage-location-analysis
title: 微信目录存放位置解析
subtitle: 对于数据库结构的总结
articleRole: main
link: '#'
---

## 总览

微信主要的目录分为数据库目录和聊天资源目录

## 根目录

1. 默认根目录名以 `xwechat_files` 为主。
2. 根目录下会包含账号目录，账号目录大多以wxid附加_xxxx的四位随机数作为文件夹名，少数以自定义微信号附加_xxxx的四位随机数作为文件夹名，还有 `all_users` 一类公共目录。
3. 对于账号目录，只需判断其是否存在 `db_storage` 文件夹即可，存在这个文件夹的就是账号目录

## 主要数据库的位置

1. `message_*.db` 位于 `db_storage/message` 目录下，文件名以 `message_` 开头，记录了所有的聊天记录信息
2. `media_*.db` 位于 `db_storage/Media` 目录下，文件名以 `media_` 开头，记录了所有的媒体文件信息
3. `contact.db` 位于 `db_storage/Contact` 目录下，记录了所有的联系人信息
4. `emoticon.db` 位于 `db_storage/emoticon` 目录下，记录了所有的表情信息
5. `message_fts.db` 位于 `db_storage/message` 目录下，记录了所有的聊天记录索引信息
6. `head_image.db` 位于 `db_storage/head_image` 目录下，记录了所有的头像信息
7. `hardlink.db` 位于 `db_storage/hardlink` 目录下，记录了所有的硬链接信息，用于快速寻找图片和视频等的本地缓存
8. `sns.db` 位于 `db_storage/sns` 目录下，记录了所有的朋友圈信息

## 目录模型

一般的目录结构如下：

```text
{root}
└── {account}
    ├── db_storage
    │   ├── session/session.db
    │   ├── message
    │   │   ├── message_*.db
    │   │   └── message_fts.db
    │   ├── emoticon
    │   │   └── emoticon.db
    │   ├── head_image
    │   │   └── head_image.db
    │   ├── hardlink
    │   │   └── hardlink.db
    │   └── sns
    │       └── sns.db
    └── msg
        ├── attach 存储了聊天接收的图片[以.dat加密存储，解密详见后文]
        ├── file 存储了聊天传输的文件[未加密]
        ├── video 存储了聊天接收的视频[未加密]
        └── migrate 不知道干什么的
```
