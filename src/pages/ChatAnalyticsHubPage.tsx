import { ArrowRight, MessageSquare, Users } from 'lucide-react'
import { useNavigate } from 'react-router-dom'
import './ChatAnalyticsHubPage.scss'
import { t } from '../i18n'

function ChatAnalyticsHubPage() {
  const navigate = useNavigate()

  return (
    <div className="analytics-hub">
      <div className="analytics-hub-inner">
        <div className="analytics-hub-hero">
          <h1 className="analytics-hub-title">{t('聊天分析')}</h1>
          <p className="analytics-hub-desc">{t('选择你要进入的分析视角，深入了解关系网络、活跃时段与消息趋势。')}</p>
        </div>

        <div className="analytics-hub-perspectives">
          <div className="analytics-hub-perspectives-label">{t('视角')}</div>

          <button
            type="button"
            className="analytics-hub-row"
            onClick={() => navigate('/analytics/private')}
          >
            <div className="analytics-hub-row-icon">
              <MessageSquare size={18} />
            </div>
            <div className="analytics-hub-row-body">
              <div className="analytics-hub-row-title">{t('私聊分析')}</div>
              <div className="analytics-hub-row-desc">{t('查看好友聊天统计、消息趋势、活跃时段与联系人排名。')}</div>
            </div>
            <ArrowRight size={16} className="analytics-hub-row-arrow" />
          </button>

          <button
            type="button"
            className="analytics-hub-row"
            onClick={() => navigate('/analytics/group')}
          >
            <div className="analytics-hub-row-icon analytics-hub-row-icon--group">
              <Users size={18} />
            </div>
            <div className="analytics-hub-row-body">
              <div className="analytics-hub-row-title">{t('群聊分析')}</div>
              <div className="analytics-hub-row-desc">{t('查看群成员信息、发言排行、活跃时段和媒体内容统计。')}</div>
            </div>
            <ArrowRight size={16} className="analytics-hub-row-arrow" />
          </button>
        </div>
      </div>
    </div>
  )
}

export default ChatAnalyticsHubPage
