import { useNavigate } from 'react-router-dom'
import { BarChart2, History, RefreshCcw } from 'lucide-react'
import { useAnalyticsStore } from '../stores/analyticsStore'
import ChatAnalysisHeader from '../components/ChatAnalysisHeader'
import './AnalyticsWelcomePage.scss'
import { t } from '../i18n'

function AnalyticsWelcomePage() {
    const navigate = useNavigate()
    const { lastLoadTime } = useAnalyticsStore()

    const handleLoadCache = () => {
        navigate('/analytics/private/view')
    }

    const handleNewAnalysis = () => {
        navigate('/analytics/private/view', { state: { forceRefresh: true } })
    }

    const formatLastTime = (ts: number | null) => {
        if (!ts) return t('无记录')
        return new Date(ts).toLocaleString()
    }

    return (
        <div className="analytics-welcome-shell">
            <ChatAnalysisHeader currentMode="private" />

            <div className="analytics-welcome-body">
                <div className="analytics-welcome-content">
                    <div className="analytics-welcome-icon">
                        <BarChart2 size={32} />
                    </div>
                    <h1>{t('私聊数据分析')}</h1>
                    <p>{t('分析你的好友聊天记录，生成详细统计报表。')}<br />{t('选择加载上次结果或开始新分析。')}</p>

                    <div className="analytics-welcome-actions">
                        <button className="analytics-welcome-card" onClick={handleLoadCache} type="button">
                            <History size={20} />
                            <div className="analytics-welcome-card-text">
                                <span className="analytics-welcome-card-title">{t('加载缓存')}</span>
                                <span className="analytics-welcome-card-meta">{t('上次更新: {v0}', { v0: formatLastTime(lastLoadTime) })}
                                </span>
                            </div>
                        </button>

                        <button className="analytics-welcome-card" onClick={handleNewAnalysis} type="button">
                            <RefreshCcw size={20} />
                            <div className="analytics-welcome-card-text">
                                <span className="analytics-welcome-card-title">{t('新的分析')}</span>
                                <span className="analytics-welcome-card-meta">{t('重新扫描并计算数据')}</span>
                            </div>
                        </button>
                    </div>
                </div>
            </div>
        </div>
    )
}

export default AnalyticsWelcomePage
