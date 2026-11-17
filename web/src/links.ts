/**
 * The address a link may open, when it is a web page's: `http:` or `https:`. The addresses of link cards, articles
 * and shared posts come from the records, not from WeFlow, and the page holds the token of the command API, so a
 * `javascript:` address, a `file:` one or a scheme that wakes a program on this computer is shown without a link
 * (#818). Preact, which renders the released page, passes any address to `href` as it is.
 */
export const safeUrl = (url?: string | null): string | undefined => (url && /^https?:\/\//i.test(url) ? url : undefined);
