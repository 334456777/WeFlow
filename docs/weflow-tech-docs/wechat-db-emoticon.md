---
id: wechat-db-emoticon
title: emoticon.db 字段详解
subtitle: 表情资源字段释义
articleRole: sub
parentId: wechat-db-structure-main
link: '#'
---
# emoticon.db 字段详解

## 数据库作用

管理表情包元数据、下载状态及自定义表情的索引记录。

## 表与字段

### `kNonStoreEmoticonTable`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| type | INTEGER 唯一索引 | 表情资源类型编码 | 微信内部扩展字段 => 表情类型枚举 |
| md5 | TEXT 唯一索引 | 表情内容 MD5 资源主检索键 | 采用 32 位十六进制表示的特征摘要，匹配时不区分大小写 => 非商店表情主键 |
| caption | TEXT | 表情文案 | 标准字符串形式的文本内容 => 表情文案 |
| product_id | TEXT 普通索引 | 表情包或商品 ID | 文本 ID => 归属包或商品标识 |
| aes_key | TEXT | 资源解密密钥 | 密钥字符串 => 表情加密下载解密键 |
| thumb_url | TEXT | 缩略图地址 | 标准格式的网络地址（URL） => 缩略图地址 |
| tp_url | TEXT | 预览图地址 | 标准格式的网络地址（URL） => 预览图地址 |
| auth_key | TEXT | 资源鉴权键 | 鉴权字符串 => 资源鉴权参数 |
| cdn_url | TEXT | CDN 下载地址 | 标准格式的网络地址（URL） => CDN 下载地址 |
| extern_url | TEXT | 外链地址 | 标准格式的网络地址（URL） => 外链优先地址 |
| extern_md5 | TEXT | 外链资源 MD5 | 标准的 32 位十六进制特征摘要 => 外链资源指纹 |
| encrypt_url | TEXT | 加密资源地址 | 标准格式的网络地址（URL） => 加密资源地址 |


### `kStoreEmoticonCaptionsTable`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| package_id_ | TEXT 唯一索引 | 表情包 ID | 表情包 ID => 文案归属包 |
| md5_ | TEXT 唯一索引 普通索引 | 表情内容 MD5 释义检索键 | 采用 32 位十六进制表示的特征摘要，匹配时不区分大小写 => 文案命中键 |
| language_ | TEXT 唯一索引 | 语言代码 | 语言代码 => 多语言区分键 |
| caption_ | TEXT | 多语言表情文案 | 标准字符串形式的文本内容 => 多语言表情文案 |


### `kStoreEmoticonFilesTable`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| package_id_ | TEXT 唯一索引 | 表情包 ID | 表情包 ID => 文件切片归属包 |
| md5_ | TEXT 唯一索引 普通索引 | 表情内容 MD5 | 32 位十六进制摘要 => 表情文件主键 |
| type_ | INTEGER | 资源类型编码 | 微信内部扩展字段 => 文件类型枚举 |
| sort_order_ | INTEGER | 包内排序序号 | 系统使用该数值进行升序排列，数值越小展示越靠前 => 包内展示顺序 |
| emoticon_size_ | INTEGER | 表情正文大小 | `>0` => 正文大小 字节 |
| emoticon_offset_ | INTEGER | 表情正文在包内偏移 | `>=0` => 正文在包内偏移 |
| thumb_size_ | INTEGER | 缩略图大小 | `>=0` => 缩略图大小 字节 |
| thumb_offset_ | INTEGER | 缩略图在包内偏移 | `>=0` => 缩略图在包内偏移 |


### `kStoreEmoticonPackageTable`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| package_id_ | TEXT 唯一索引 | 表情包 ID | 表情包 ID => 包主键 |
| package_name_ | TEXT | 表情包名称 | 标准字符串形式的文本内容 => 包名称 |
| payment_status_ | INTEGER | 付费状态 | 微信内部扩展字段 => 付费状态枚举 |
| download_status_ | INTEGER | 下载状态 | 微信内部扩展字段 => 下载状态枚举 |
| install_time_ | INTEGER | 安装时间戳 | 记录事件发生的 Unix 时间戳数值 => 安装时间 |
| remove_time_ | INTEGER | 卸载时间戳 | 记录事件发生的 Unix 时间戳数值 => 卸载时间 |
| sort_order_ | INTEGER | 展示排序 | 用于前端面板界面的展示排序参考值 => 面板展示顺序 |
| introduction_ | TEXT | 简介文本 | 标准字符串形式的文本内容 => 简介 |
| full_description_ | TEXT | 完整说明文本 | 标准字符串形式的文本内容 => 完整说明 |
| copyright_ | TEXT | 版权信息 | 标准字符串形式的文本内容 => 版权信息 |
| author_ | TEXT | 作者信息 | 标准字符串形式的文本内容 => 作者信息 |
| store_icon_url_ | TEXT | 商店图标地址 | 标准格式的网络地址（URL） => 商店图标 |
| panel_url_ | TEXT | 面板资源地址 | 标准格式的网络地址（URL） => 面板资源地址 |


### `kCustomEmoticonOrderTable`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| md5 | TEXT 唯一索引 | 自定义表情排序键 | 32 位十六进制摘要 => 自定义表情排序键 |


### `kFavEmoticonOrderTable`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| md5 | TEXT 唯一索引 | 收藏表情排序键 | 32 位十六进制摘要 => 收藏表情排序键 |


### `kExpressRecentUseEemoticonTable`

| 字段 | 结构 | 作用与实际用途 | 详细描述 |
|---|---|---|---|
| Key | TEXT 主键 唯一索引 | 最近使用配置键 | 配置键名 => 最近使用记录主键 |
| ValueInt64 | INTEGER | 整型配置值 | 整型值 => 整型配置承载位 |
| ValueDouble | REAL | 浮点配置值 | 浮点值 => 浮点配置承载位 |
| ValueStdStr | TEXT | 字符串配置值 | 字符串值 => 字符串配置承载位 |
| ValueBlob | BLOB | 微信内部扩展字段 | 微信内部扩展字段 => 二进制配置承载位 |

## 字段取值详解

#### knonstoreemoticontable-values
对应表 `kNonStoreEmoticonTable`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| type | 微信内部扩展字段 | 表情类型枚举 |
| md5 | 采用 32 位十六进制表示的特征摘要，匹配时不区分大小写 | 非商店表情主键 |
| caption | 标准字符串形式的文本内容 | 表情文案 |
| product_id | 文本 ID | 归属包或商品标识 |
| aes_key | 密钥字符串 | 表情加密下载解密键 |
| thumb_url | 标准格式的网络地址（URL） | 缩略图地址 |
| tp_url | 标准格式的网络地址（URL） | 预览图地址 |
| auth_key | 鉴权字符串 | 资源鉴权参数 |
| cdn_url | 标准格式的网络地址（URL） | CDN 下载地址 |
| extern_url | 标准格式的网络地址（URL） | 外链优先地址 |
| extern_md5 | 标准的 32 位十六进制特征摘要 | 外链资源指纹 |
| encrypt_url | 标准格式的网络地址（URL） | 加密资源地址 |

#### kstoreemoticoncaptionstable-values
对应表 `kStoreEmoticonCaptionsTable`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| package_id_ | 表情包 ID | 文案归属包 |
| md5_ | 采用 32 位十六进制表示的特征摘要，匹配时不区分大小写 | 文案命中键 |
| language_ | 语言代码 | 多语言区分键 |
| caption_ | 标准字符串形式的文本内容 | 多语言表情文案 |

#### kstoreemoticonfilestable-values
对应表 `kStoreEmoticonFilesTable`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| package_id_ | 表情包 ID | 文件切片归属包 |
| md5_ | 32 位十六进制摘要 | 表情文件主键 |
| type_ | 微信内部扩展字段 | 文件类型枚举 |
| sort_order_ | 系统使用该数值进行升序排列，数值越小展示越靠前 | 包内展示顺序 |
| emoticon_size_ | `>0` | 正文大小 字节 |
| emoticon_offset_ | `>=0` | 正文在包内偏移 |
| thumb_size_ | `>=0` | 缩略图大小 字节 |
| thumb_offset_ | `>=0` | 缩略图在包内偏移 |

#### kstoreemoticonpackagetable-values
对应表 `kStoreEmoticonPackageTable`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| package_id_ | 表情包 ID | 包主键 |
| package_name_ | 标准字符串形式的文本内容 | 包名称 |
| payment_status_ | 微信内部扩展字段 | 付费状态枚举 |
| download_status_ | 微信内部扩展字段 | 下载状态枚举 |
| install_time_ | 记录事件发生的 Unix 时间戳数值 | 安装时间 |
| remove_time_ | 记录事件发生的 Unix 时间戳数值 | 卸载时间 |
| sort_order_ | 用于前端面板界面的展示排序参考值 | 面板展示顺序 |
| introduction_ | 标准字符串形式的文本内容 | 简介 |
| full_description_ | 标准字符串形式的文本内容 | 完整说明 |
| copyright_ | 标准字符串形式的文本内容 | 版权信息 |
| author_ | 标准字符串形式的文本内容 | 作者信息 |
| store_icon_url_ | 标准格式的网络地址（URL） | 商店图标 |
| panel_url_ | 标准格式的网络地址（URL） | 面板资源地址 |

#### kcustomemoticonordertable-values
对应表 `kCustomEmoticonOrderTable`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| md5 | 32 位十六进制摘要 | 自定义表情排序键 |

#### kfavemoticonordertable-values
对应表 `kFavEmoticonOrderTable`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| md5 | 32 位十六进制摘要 | 收藏表情排序键 |

#### kexpressrecentuseeemoticontable-values
对应表 `kExpressRecentUseEemoticonTable`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| Key | 配置键名 | 最近使用记录主键 |
| ValueInt64 | 整型值 | 整型配置承载位 |
| ValueDouble | 浮点值 | 浮点配置承载位 |
| ValueStdStr | 字符串值 | 字符串配置承载位 |
| ValueBlob | 微信内部扩展字段 | 二进制配置承载位 |
