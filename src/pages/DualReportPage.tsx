import { useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { Loader2, Search, Users } from 'lucide-react'
import './DualReportPage.scss'
import { t } from '../i18n'

interface ContactRanking {
  username: string
  displayName: string
  avatarUrl?: string
  wechatId?: string
  messageCount: number
  sentCount: number
  receivedCount: number
  lastMessageTime?: number | null
}

function DualReportPage() {
  const navigate = useNavigate()
  const [year] = useState<number>(() => {
    const params = new URLSearchParams(window.location.hash.split('?')[1] || '')
    const yearParam = params.get('year')
    const parsedYear = yearParam ? parseInt(yearParam, 10) : 0
    return Number.isNaN(parsedYear) ? 0 : parsedYear
  })
  const [rankings, setRankings] = useState<ContactRanking[]>([])
  const [isLoading, setIsLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [keyword, setKeyword] = useState('')

  useEffect(() => {
    void loadRankings(year)
  }, [year])

  const loadRankings = async (reportYear: number) => {
    setIsLoading(true)
    setLoadError(null)
    try {
      const isAllTime = reportYear <= 0
      const beginTimestamp = isAllTime ? 0 : Math.floor(new Date(reportYear, 0, 1).getTime() / 1000)
      const endTimestamp = isAllTime ? 0 : Math.floor(new Date(reportYear, 11, 31, 23, 59, 59).getTime() / 1000)
      const result = await window.electronAPI.analytics.getContactRankings(200, beginTimestamp, endTimestamp)
      if (result.success && result.data) {
        setRankings(result.data)
      } else {
        setLoadError(result.error || t('加载好友列表失败'))
      }
    } catch (e) {
      setLoadError(String(e))
    } finally {
      setIsLoading(false)
    }
  }

  const yearLabel = year === 0 ? t('全部时间') : t('{year}年', { year: year })

  const filteredRankings = useMemo(() => {
    if (!keyword.trim()) return rankings
    const q = keyword.trim().toLowerCase()
    return rankings.filter((item) => {
      const wechatId = (item.wechatId || '').toLowerCase()
      return item.displayName.toLowerCase().includes(q) || wechatId.includes(q)
    })
  }, [rankings, keyword])

  const handleSelect = (username: string) => {
    const yearParam = year === 0 ? 0 : year
    navigate(`/dual-report/view?username=${encodeURIComponent(username)}&year=${yearParam}`)
  }

  if (isLoading) {
    return (
      <div className="dual-report-page loading">
        <Loader2 size={32} className="spin" />
        <p>{t('正在加载聊天排行...')}</p>
      </div>
    )
  }

  if (loadError) {
    return (
      <div className="dual-report-page loading">
        <p>{t('加载失败：{loadError}', { loadError: loadError })}</p>
      </div>
    )
  }

  return (
    <div className="dual-report-page">
      <div className="page-header">
        <div>
          <h1>{t('双人年度报告')}</h1>
          <p>{t('选择一位好友，生成你们的专属聊天报告')}</p>
        </div>
        <div className="year-badge">
          <Users size={14} />
          <span>{yearLabel}</span>
        </div>
      </div>

      <div className="search-bar">
        <Search size={16} />
        <input
          value={keyword}
          onChange={(e) => setKeyword(e.target.value)}
          placeholder={t('搜索好友（昵称/微信号）')}
        />
      </div>

      <div className="ranking-list">
        {filteredRankings.map((item, index) => (
          <button
            key={item.username}
            className="ranking-item"
            onClick={() => handleSelect(item.username)}
          >
            <span className={`rank-badge ${index < 3 ? 'top' : ''}`}>{index + 1}</span>
            <div className="avatar">
              {item.avatarUrl
                ? <img src={item.avatarUrl} alt={item.displayName} />
                : <span>{item.displayName.slice(0, 1) || '?'}</span>
              }
            </div>
            <div className="info">
              <div className="name">{item.displayName}</div>
              <div className="sub">{item.wechatId || t('未设置微信号')}</div>
            </div>
            <div className="meta">
              <div className="count">{t('{v0} 条', { v0: item.messageCount.toLocaleString() })}</div>
              <div className="hint">{t('总消息')}</div>
            </div>
          </button>
        ))}
        {filteredRankings.length === 0 ? (
          <div className="empty">{t('没有匹配的好友')}</div>
        ) : null}
      </div>
    </div>
  )
}

export default DualReportPage
