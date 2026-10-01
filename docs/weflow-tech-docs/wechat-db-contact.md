---
id: wechat-db-contact
title: contact.db 字段详解
subtitle: 联系人与群关系字段释义
articleRole: sub
parentId: wechat-db-structure-main
link: '#'
---
# contact.db 字段详解

## 数据库作用

存储联系人、群聊成员及公众号的详细资料，包含权限标记、备注属性及底层关系映射。

## 表与字段

### `contact`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [id](#contact-values) | INTEGER 主键 | 联系人记录主键 |
| [username](#contact-values) | TEXT | 联系人唯一标识 单聊对象 群标识 公众号标识都使用该键 |
| [local_type](#contact-values) | INTEGER 普通索引 | 联系人类型编码 常用于区分好友与非好友 |
| [alias](#contact-values) | TEXT | 微信号或账号别名 |
| [encrypt_username](#contact-values) | TEXT | 微信内部扩展字段 |
| [flag](#contact-values) | INTEGER | 联系人状态位 可承载折叠等状态 |
| [delete_flag](#contact-values) | INTEGER | 删除状态标记 |
| [verify_flag](#contact-values) | INTEGER | 认证状态标记 |
| [remark](#contact-values) | TEXT | 备注名 展示优先级通常高于昵称 |
| [remark_quan_pin](#contact-values) | TEXT | 备注全拼 检索键 |
| [remark_pin_yin_initial](#contact-values) | TEXT | 备注首字母 检索键 |
| [nick_name](#contact-values) | TEXT | 昵称 |
| [pin_yin_initial](#contact-values) | TEXT | 昵称首字母 检索键 |
| [quan_pin](#contact-values) | TEXT | 昵称全拼 检索键 |
| [big_head_url](#contact-values) | TEXT | 大头像地址 |
| [small_head_url](#contact-values) | TEXT | 小头像地址 |
| [head_img_md5](#contact-values) | TEXT | 头像内容 MD5 摘要 |
| [chat_room_notify](#contact-values) | INTEGER | 群通知相关状态位 |
| [is_in_chat_room](#contact-values) | INTEGER | 群关系标记 表示该联系人是否处于群体系 |
| [description](#contact-values) | TEXT | 联系人简介文本 |
| [extra_buffer](#contact-values) | BLOB | 微信内部扩展字段 |
| [chat_room_type](#contact-values) | INTEGER | 微信内部扩展字段 |


### `stranger`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [id](#stranger-values) | INTEGER 主键 | 陌生人记录主键 |
| [username](#stranger-values) | TEXT | 陌生人唯一标识 |
| [local_type](#stranger-values) | INTEGER 普通索引 | 陌生人类型编码 |
| [alias](#stranger-values) | TEXT | 微信号或账号别名 |
| [encrypt_username](#stranger-values) | TEXT | 微信内部扩展字段 |
| [flag](#stranger-values) | INTEGER | 状态位 |
| [delete_flag](#stranger-values) | INTEGER | 删除状态标记 |
| [verify_flag](#stranger-values) | INTEGER | 认证状态标记 |
| [remark](#stranger-values) | TEXT | 备注名 |
| [remark_quan_pin](#stranger-values) | TEXT | 备注全拼 检索键 |
| [remark_pin_yin_initial](#stranger-values) | TEXT | 备注首字母 检索键 |
| [nick_name](#stranger-values) | TEXT | 昵称 |
| [pin_yin_initial](#stranger-values) | TEXT | 昵称首字母 检索键 |
| [quan_pin](#stranger-values) | TEXT | 昵称全拼 检索键 |
| [big_head_url](#stranger-values) | TEXT | 大头像地址 |
| [small_head_url](#stranger-values) | TEXT | 小头像地址 |
| [head_img_md5](#stranger-values) | TEXT | 头像内容 MD5 摘要 |
| [chat_room_notify](#stranger-values) | INTEGER | 群通知相关状态位 |
| [is_in_chat_room](#stranger-values) | INTEGER | 群关系标记 |
| [description](#stranger-values) | TEXT | 简介文本 |
| [extra_buffer](#stranger-values) | BLOB | 微信内部扩展字段 |
| [chat_room_type](#stranger-values) | INTEGER | 微信内部扩展字段 |


### `chat_room`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [id](#chatroom-values) | INTEGER 主键 | 群记录主键 |
| [username](#chatroom-values) | TEXT | 群唯一标识 通常为 `xxx@chatroom` |
| [owner](#chatroom-values) | TEXT | 群主标识 |
| [ext_buffer](#chatroom-values) | BLOB | 微信内部扩展字段 |


### `chatroom_member`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [room_id](#chatroommember-values) | INTEGER 唯一索引 普通索引 | 群 rowid 外键 指向 `name2id.rowid` |
| [member_id](#chatroommember-values) | INTEGER 唯一索引 普通索引 | 成员 rowid 外键 指向 `name2id.rowid` |


### `name2id`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [username](#name2id-values) | TEXT 主键 唯一索引 | 用户名到 rowid 的基础映射键 群成员关系依赖该映射 |


### `encrypt_name2id`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [username](#encryptname2id-values) | TEXT 主键 唯一索引 | 加密用户名到 rowid 的映射键 |


### `contact_label`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [label_id_](#contactlabel-values) | INTEGER 主键 | 标签 ID |
| [label_name_](#contactlabel-values) | TEXT | 标签名称 |
| [sort_order_](#contactlabel-values) | INTEGER | 标签排序权重 |


### `ticket_info`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [id](#ticketinfo-values) | INTEGER 主键 | 记录主键 |
| [ticket](#ticketinfo-values) | TEXT | 联系人验证票据 |


### `stranger_ticket_info`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [id](#strangerticketinfo-values) | INTEGER 主键 | 记录主键 |
| [ticket](#strangerticketinfo-values) | TEXT | 陌生人验证票据 |


### `biz_info`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [id](#bizinfo-values) | INTEGER 主键 | 公众号资料记录主键 |
| [username](#bizinfo-values) | TEXT | 公众号标识 |
| [type](#bizinfo-values) | INTEGER | 账号类型编码 |
| [accept_type](#bizinfo-values) | INTEGER | 接收类型配置 |
| [child_type](#bizinfo-values) | INTEGER | 子类型编码 |
| [version](#bizinfo-values) | INTEGER | 资料版本号 |
| [external_info](#bizinfo-values) | TEXT | 外部资料文本 |
| [brand_info](#bizinfo-values) | TEXT | 品牌资料文本 |
| [brand_icon_url](#bizinfo-values) | TEXT | 品牌图标地址 |
| [brand_list](#bizinfo-values) | TEXT | 品牌列表文本 |
| [brand_flag](#bizinfo-values) | INTEGER | 品牌状态位 |
| [belong](#bizinfo-values) | TEXT | 归属信息 |
| [ext_buffer](#bizinfo-values) | BLOB | 微信内部扩展字段 |
| [home_url](#bizinfo-values) | TEXT | 主页地址 |
| [sync_version](#bizinfo-values) | TEXT | 同步版本标识 |


### `biz_profile`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [username](#bizprofile-values) | TEXT | 公众号标识 |
| [service_type](#bizprofile-values) | INTEGER | 服务类型编码 |
| [article_count](#bizprofile-values) | INTEGER | 历史文章数量 |
| [friend_sub_count](#bizprofile-values) | INTEGER | 关注人数 |
| [is_subscribe](#bizprofile-values) | INTEGER | 当前账号是否已关注 |
| [offset](#bizprofile-values) | TEXT | 分页游标 |
| [time_stamp](#bizprofile-values) | INTEGER 普通索引 | 更新时间戳 |
| [is_end](#bizprofile-values) | INTEGER | 是否到达列表末尾 |
| [resp_buffer](#bizprofile-values) | BLOB | 微信内部扩展字段 |


### `biz_session_feeds`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [username](#bizsessionfeeds-values) | TEXT 唯一索引 | 公众号会话标识 |
| [showname](#bizsessionfeeds-values) | TEXT | 展示名称 |
| [desc](#bizsessionfeeds-values) | TEXT | 摘要描述 |
| [type](#bizsessionfeeds-values) | INTEGER | 会话类型编码 |
| [unread_count](#bizsessionfeeds-values) | INTEGER | 未读计数 |
| [update_time](#bizsessionfeeds-values) | INTEGER | 更新时间戳 |
| [create_time](#bizsessionfeeds-values) | INTEGER | 创建时间戳 |
| [biz_attr_version](#bizsessionfeeds-values) | INTEGER | 业务属性版本号 |


### `openim_acct_type`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [lang_id](#openimaccttype-values) | INTEGER 联合主键 唯一索引 | 语言 ID |
| [acc_type_id](#openimaccttype-values) | TEXT 联合主键 唯一索引 | 账号类型 ID |
| [update_time](#openimaccttype-values) | INTEGER | 更新时间戳 |
| [ext_buffer](#openimaccttype-values) | BLOB | 微信内部扩展字段 |


### `openim_appid`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [lang_id](#openimappid-values) | INTEGER 联合主键 唯一索引 | 语言 ID |
| [app_id](#openimappid-values) | TEXT 联合主键 唯一索引 | 应用 ID |
| [acct_type_id](#openimappid-values) | TEXT | 账号类型引用 |
| [update_time](#openimappid-values) | INTEGER | 更新时间戳 |
| [ext_buffer](#openimappid-values) | BLOB | 微信内部扩展字段 |


### `openim_wording`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [lang_id](#openimwording-values) | INTEGER 联合主键 唯一索引 | 语言 ID |
| [app_id](#openimwording-values) | TEXT 联合主键 唯一索引 | 应用 ID |
| [wording_id](#openimwording-values) | TEXT 联合主键 唯一索引 | 文案 ID |
| [wording](#openimwording-values) | TEXT | 文案正文 |
| [pinyin](#openimwording-values) | TEXT | 拼音检索键 |
| [quan_pin](#openimwording-values) | TEXT | 全拼检索键 |
| [update_time](#openimwording-values) | INTEGER | 更新时间戳 |
| [ext_buffer](#openimwording-values) | BLOB | 微信内部扩展字段 |


### `chat_room_info_detail`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [room_id_](#chatroominfodetail-values) | INTEGER 主键 | 群 rowid |
| [username_](#chatroominfodetail-values) | TEXT | 群标识 |
| [announcement_](#chatroominfodetail-values) | TEXT | 群公告正文 |
| [announcement_editor_](#chatroominfodetail-values) | TEXT | 群公告编辑者 |
| [announcement_publish_time_](#chatroominfodetail-values) | INTEGER | 群公告发布时间 |
| [chat_room_status_](#chatroominfodetail-values) | INTEGER | 群状态位 |
| [room_top_msg_closed_id_list_text_](#chatroominfodetail-values) | TEXT | 微信内部扩展字段 |
| [xml_announcement_](#chatroominfodetail-values) | TEXT | 公告 XML 原文 |
| [ext_buffer_](#chatroominfodetail-values) | BLOB | 微信内部扩展字段 |


### `oplog`

| 字段 | 结构 | 作用与实际用途 |
|---|---|---|
| [id](#oplog-values) | INTEGER 主键 | 操作日志记录主键 |
| [buffer](#oplog-values) | BLOB | 微信内部扩展字段 |

## 字段取值详解

#### contact-values
对应表 `contact`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| id | 系统自动生成的递增式正整数主键 | 联系人记录主键 |
| username | 以 `@chatroom` 结尾 | 群对象标识 |
| username | 以 `gh_` 开头 | 公众号对象标识 |
| username | 其他常规值 | 个人联系人或系统账号 |
| local_type | `1` | 好友关系 |
| local_type | `0` 且 `quan_pin` 非空 | 曾经好友或已变更关系联系人 |
| alias | 标准字符串形式的文本内容 | 微信号或账号别名 |
| encrypt_username | 微信内部扩展字段 | 微信内部预留的扩展字段 |
| flag | `flag & 0x10000000 != 0` | 折叠状态为真 |
| flag | 其他位组合 | 其他状态位 暂未完整公开 |
| delete_flag | 微信内部扩展字段 | 删除状态枚举 暂未完整公开 |
| verify_flag | 微信内部扩展字段 | 认证状态枚举 暂未完整公开 |
| remark | 非空优先 | 展示名优先级最高 |
| remark_quan_pin | 拼音文本 | 备注全拼检索 |
| remark_pin_yin_initial | 首字母文本 | 备注首字母检索 |
| nick_name | 非空回退 | remark 为空时作为展示名 |
| pin_yin_initial | 首字母文本 | 昵称首字母检索 |
| quan_pin | 拼音文本 | 昵称全拼检索 |
| big_head_url | 非空优先 | 头像地址优先级高于 `small_head_url` |
| small_head_url | big 为空时使用 | 头像地址回退值 |
| head_img_md5 | 底层通过计算得出的 32 位消息摘要，作为文件内容的唯一数字指纹 | 头像内容指纹 |
| chat_room_notify | 微信内部扩展字段 | 群通知相关状态位 |
| is_in_chat_room | `1` 或 `0` | 是否处于群体系关系 |
| description | 标准字符串形式的文本内容 | 联系人简介 |
| extra_buffer | protobuf field `12` 非 `0` | 免打扰状态为真 |
| extra_buffer | 其他字段 | 微信内部扩展语义，目前尚未完整公开 |
| chat_room_type | 微信内部扩展字段 | 群类型枚举 暂未完整公开 |

#### stranger-values
对应表 `stranger`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| id | 系统自动生成的递增式正整数主键 | 陌生人记录主键 |
| username | 账号标识文本 | 陌生人唯一键 |
| local_type | 微信内部扩展字段 | 陌生人关系类型枚举 |
| alias | 标准字符串形式的文本内容 | 陌生人别名 |
| encrypt_username | 微信内部扩展字段 | 微信内部预留的扩展字段 |
| flag | 微信内部扩展字段 | 状态位组合 |
| delete_flag | 微信内部扩展字段 | 删除状态枚举 |
| verify_flag | 微信内部扩展字段 | 认证状态枚举 |
| remark | 标准字符串形式的文本内容 | 备注名 |
| remark_quan_pin | 拼音文本 | 备注全拼检索 |
| remark_pin_yin_initial | 首字母文本 | 备注首字母检索 |
| nick_name | 标准字符串形式的文本内容 | 昵称 |
| pin_yin_initial | 首字母文本 | 昵称首字母检索 |
| quan_pin | 拼音文本 | 昵称全拼检索 |
| big_head_url | 非空优先 | 头像优先地址 |
| small_head_url | big 为空时使用 | 头像回退地址 |
| head_img_md5 | 底层通过计算得出的 32 位消息摘要，作为文件内容的唯一数字指纹 | 头像内容指纹 |
| chat_room_notify | 微信内部扩展字段 | 群通知相关状态位 |
| is_in_chat_room | `1` 或 `0` | 是否参与群关系 |
| description | 标准字符串形式的文本内容 | 简介文本 |
| extra_buffer | 微信内部扩展字段 | 微信内部预留的扩展字段 |
| chat_room_type | 微信内部扩展字段 | 群类型枚举 |

#### chatroom-values
对应表 `chat_room`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| id | 系统自动生成的递增式正整数主键 | 群记录主键 |
| username | 以 `@chatroom` 结尾 | 群唯一标识 |
| owner | 用户名文本 | 群主标识 |
| ext_buffer | 微信内部扩展字段 | 群扩展信息载荷 |

#### chatroommember-values
对应表 `chatroom_member`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| room_id | 命中 `name2id.rowid` | 群对象外键 |
| member_id | 命中 `name2id.rowid` | 成员对象外键 |

#### name2id-values
对应表 `name2id`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| username | 唯一文本 | 用户名到 rowid 的基础映射 |

#### encryptname2id-values
对应表 `encrypt_name2id`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| username | 唯一文本 | 加密用户名到 rowid 的映射 |

#### contactlabel-values
对应表 `contact_label`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| label_id_ | 正整数 | 标签主键 |
| label_name_ | 标准字符串形式的文本内容 | 标签名称 |
| sort_order_ | 数值越小通常越靠前 | 标签排序权重 |

#### ticketinfo-values
对应表 `ticket_info`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| id | 系统自动生成的递增式正整数主键 | 票据记录主键 |
| ticket | 票据字符串 | 联系人验证票据 |

#### strangerticketinfo-values
对应表 `stranger_ticket_info`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| id | 系统自动生成的递增式正整数主键 | 票据记录主键 |
| ticket | 票据字符串 | 陌生人验证票据 |

#### bizinfo-values
对应表 `biz_info`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| id | 系统自动生成的递增式正整数主键 | 公众号资料主键 |
| username | 以 `gh_` 开头常见 | 公众号标识 |
| type | 微信内部扩展字段 | 公众号账号类型枚举 |
| accept_type | 微信内部扩展字段 | 接收类型配置枚举 |
| child_type | 微信内部扩展字段 | 子类型枚举 |
| version | 递增版本号 | 资料版本控制 |
| external_info | 标准字符串形式的文本内容 | 外部资料 |
| brand_info | 标准字符串形式的文本内容 | 品牌资料 |
| brand_icon_url | 标准格式的网络地址（URL） | 品牌图标地址 |
| brand_list | 标准字符串形式的文本内容 | 品牌列表信息 |
| brand_flag | 微信内部扩展字段 | 品牌状态位 |
| belong | 标准字符串形式的文本内容 | 归属信息 |
| ext_buffer | 微信内部扩展字段 | 微信内部预留的扩展字段 |
| home_url | 标准格式的网络地址（URL） | 主页地址 |
| sync_version | 文本版本号 | 同步版本标识 |

#### bizprofile-values
对应表 `biz_profile`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| username | 公众号标识 | 资料归属账号 |
| service_type | 微信内部扩展字段 | 公众号服务类型 |
| article_count | `>=0` | 历史文章数量 |
| friend_sub_count | `>=0` | 关注人数 |
| is_subscribe | `1` | 当前账号已关注 |
| is_subscribe | `0` | 当前账号未关注 |
| offset | 分页游标字符串 | 拉取下一批资料的游标 |
| time_stamp | 记录事件发生的 Unix 时间戳数值 | 资料更新时间 |
| is_end | `1` | 已到列表末尾 |
| is_end | `0` | 仍有后续分页 |
| resp_buffer | 微信内部扩展字段 | 微信内部预留的扩展字段 |

#### bizsessionfeeds-values
对应表 `biz_session_feeds`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| username | 公众号标识 | 会话归属账号 |
| showname | 标准字符串形式的文本内容 | 展示名称 |
| desc | 标准字符串形式的文本内容 | 摘要描述 |
| type | 微信内部扩展字段 | 会话类型枚举 |
| unread_count | `0` | 当前无未读 |
| unread_count | `>0` | 当前有未读 |
| update_time | 记录事件发生的 Unix 时间戳数值 | 最近更新时间 |
| create_time | 记录事件发生的 Unix 时间戳数值 | 记录创建时间 |
| biz_attr_version | 递增版本号 | 业务属性版本控制 |

#### openimaccttype-values
对应表 `openim_acct_type`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| lang_id | 语言编号 | 语言维度主键 |
| acc_type_id | 类型字符串 | 账号类型主键 |
| update_time | 记录事件发生的 Unix 时间戳数值 | 更新时间 |
| ext_buffer | 微信内部扩展字段 | 微信内部预留的扩展字段 |

#### openimappid-values
对应表 `openim_appid`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| lang_id | 语言编号 | 语言维度主键 |
| app_id | 应用标识 | 应用主键 |
| acct_type_id | 类型字符串 | 关联账号类型 |
| update_time | 记录事件发生的 Unix 时间戳数值 | 更新时间 |
| ext_buffer | 微信内部扩展字段 | 微信内部预留的扩展字段 |

#### openimwording-values
对应表 `openim_wording`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| lang_id | 语言编号 | 语言维度主键 |
| app_id | 应用标识 | 应用维度主键 |
| wording_id | 文案标识 | 文案主键 |
| wording | 标准字符串形式的文本内容 | 文案正文 |
| pinyin | 拼音文本 | 拼音检索 |
| quan_pin | 全拼文本 | 全拼检索 |
| update_time | 记录事件发生的 Unix 时间戳数值 | 更新时间 |
| ext_buffer | 微信内部扩展字段 | 微信内部预留的扩展字段 |

#### chatroominfodetail-values
对应表 `chat_room_info_detail`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| room_id_ | 命中群 rowid | 群对象主键 |
| username_ | 以 `@chatroom` 结尾 | 群标识 |
| announcement_ | 标准字符串形式的文本内容 | 群公告正文 |
| announcement_editor_ | 用户名文本 | 公告编辑者 |
| announcement_publish_time_ | 记录事件发生的 Unix 时间戳数值 | 公告发布时间 |
| chat_room_status_ | 微信内部扩展字段 | 群状态位 |
| room_top_msg_closed_id_list_text_ | 微信内部扩展字段 | 微信内部预留的扩展字段 |
| xml_announcement_ | XML 文本 | 公告原文 |
| ext_buffer_ | 微信内部扩展字段 | 微信内部预留的扩展字段 |

#### oplog-values
对应表 `oplog`

| 字段 | 取值 | 用途与实际意义 |
|---|---|---|
| id | 系统自动生成的递增式正整数主键 | 操作日志主键 |
| buffer | 微信内部扩展字段 | 操作日志二进制载荷 |
