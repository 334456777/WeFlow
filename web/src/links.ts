/**
 * 链接可以打开的地址：网页的地址（`http:` 或 `https:`）原样返回，其他地址返回 `undefined`。
 * 聊天里的链接卡片、公众号文章和朋友圈分享的链接来自读到的记录而不是 WeFlow，而页面持有命令 API 的 token，所以
 * `javascript:`、`file:` 和会唤起本机程序的协议都只显示文字、不加链接（#818）。
 * 渲染发布版页面的 Preact 会把任何地址原样交给 `href`。
 */
export const safeUrl = (url?: string | null): string | undefined => (url && /^https?:\/\//i.test(url) ? url : undefined);
