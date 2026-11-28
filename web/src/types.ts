// 各页面共用的类型：导航里的页面，以及在网页里完成的导出的结果。

/** 页面所属的类别：朋友圈和支付记录各自一类，其余页面都属于聊天。设置页显示最后打开的那一类的卡片。 */
export type Category = 'chat' | 'moments' | 'payments';
/** 导航里的全部页面。 */
export type Navigation = Category | 'contacts' | 'resources' | 'analytics' | 'reports' | 'footprint' | 'insights' | 'export' | 'backup' | 'settings';
/** 网页自己写出的导出（支付记录）的格式。 */
export type ExportFormat = 'HTML' | 'JSON' | 'Excel';

/** 完成的导出，供下载。 */
export interface ExportResult {
  fileName: string;
  /** 其中的记录数。 */
  count: number;
  downloadUrl: string;
}
