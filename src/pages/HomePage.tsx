import './HomePage.scss'
import { t } from '../i18n'

function HomePage() {
  return (
    <div className="home-page">
      <div className="home-content">
        <h1 className="home-title">WeFlow</h1>
        <p className="home-subtitle">{t('每一条消息的背后，都藏着一段温暖的时光')}</p>
      </div>
    </div>
  )
}

export default HomePage
