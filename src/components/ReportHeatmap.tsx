import React from 'react'
import './ReportComponents.scss'
import { t } from '../i18n'

interface ReportHeatmapProps {
    data: number[][]
}

const ReportHeatmap: React.FC<ReportHeatmapProps> = ({ data }) => {
    if (!data || data.length === 0) return null

    const maxHeat = Math.max(...data.flat())
    const weekLabels = [t('周一'), t('周二'), t('周三'), t('周四'), t('周五'), t('周六'), t('周日')]

    return (
        <div className="heatmap-wrapper">
            <div className="heatmap-header">
                <div></div>
                <div className="time-labels">
                    {[0, 6, 12, 18].map(h => (
                        <span key={h} style={{ gridColumn: h + 1 }}>{h}</span>
                    ))}
                </div>
            </div>
            <div className="heatmap">
                <div className="heatmap-week-col">
                    {weekLabels.map(w => <div key={w} className="week-label">{w}</div>)}
                </div>
                <div className="heatmap-grid">
                    {data.map((row, wi) =>
                        row.map((val, hi) => {
                            const alpha = maxHeat > 0 ? (val / maxHeat * 0.85 + 0.1).toFixed(2) : '0.1'
                            return (
                                <div
                                    key={`${wi}-${hi}`}
                                    className="h-cell"
                                    style={{
                                        backgroundColor: 'var(--primary)',
                                        opacity: alpha
                                    }}
                                    title={t('{v0} {hi}:00 - {val}条', { v0: weekLabels[wi], hi: hi, val: val })}
                                />
                            )
                        })
                    )}
                </div>
            </div>
        </div>
    )
}

export default ReportHeatmap
