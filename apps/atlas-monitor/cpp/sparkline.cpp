#include "sparkline.h"

#include <QPainter>
#include <QPainterPath>

#include <algorithm>
#include <cmath>

namespace {
// Equal, NaN to NaN too: a card left unread would otherwise repaint its
// empty line every tick.
bool same(const QList<qreal> &a, const QList<qreal> &b)
{
    if (a.size() != b.size()) {
        return false;
    }
    for (qsizetype i = 0; i < a.size(); ++i) {
        if (a[i] != b[i] && !(std::isnan(a[i]) && std::isnan(b[i]))) {
            return false;
        }
    }
    return true;
}
} // namespace

Sparkline::Sparkline(QQuickItem *parent)
    : QQuickPaintedItem(parent)
{
    connect(this, &Sparkline::styleChanged, this, [this] { update(); });
}

void Sparkline::setValues(const QList<qreal> &values)
{
    if (same(values, m_values)) {
        return;
    }
    m_values = values;
    Q_EMIT valuesChanged();
    update();
}

void Sparkline::paint(QPainter *p)
{
    const int length = std::max(2, m_length);
    const qsizetype n = std::min<qsizetype>(m_values.size(), length);
    const double w = width();
    const double h = height();
    if (n == 0 || w <= 0 || h <= 2) {
        return;
    }
    const qsizetype first = m_values.size() - n;

    double top = m_maximum;
    if (top <= 0) {
        double high = 0;
        for (qsizetype i = first; i < m_values.size(); ++i) {
            if (std::isfinite(m_values[i])) {
                high = std::max(high, m_values[i]);
            }
        }
        top = std::max(high * 1.25, m_minimumScale);
    }
    if (top <= 0) {
        top = 1;
    }

    // The line's half width stays inside the item at the top and bottom.
    const double line = 1.5;
    const double plotTop = line / 2;
    const double plotH = h - line;
    const double step = w / (length - 1);
    const auto xAt = [&](qsizetype i) { return w - double(m_values.size() - 1 - i) * step; };
    const auto yAt = [&](double v) { return plotTop + plotH * (1 - std::clamp(v / top, 0.0, 1.0)); };

    // One path per unbroken run of samples.
    QPainterPath stroke;
    QPainterPath fill;
    qsizetype runStart = -1;
    const auto closeRun = [&](qsizetype end) {
        if (runStart < 0) {
            return;
        }
        // A lone sample (the first tick, or one between gaps): a dot, from
        // the pen's round cap.
        if (end == runStart) {
            stroke.lineTo(stroke.currentPosition() + QPointF(0.01, 0));
        }
        fill.lineTo(xAt(end), h);
        fill.lineTo(xAt(runStart), h);
        fill.closeSubpath();
        runStart = -1;
    };
    for (qsizetype i = first; i < m_values.size(); ++i) {
        const double v = m_values[i];
        if (!std::isfinite(v)) {
            closeRun(i - 1);
            continue;
        }
        const QPointF pt(xAt(i), yAt(v));
        if (runStart < 0) {
            runStart = i;
            stroke.moveTo(pt);
            fill.moveTo(pt);
        } else {
            stroke.lineTo(pt);
            fill.lineTo(pt);
        }
    }
    closeRun(m_values.size() - 1);

    p->setRenderHint(QPainter::Antialiasing, true);
    QColor soft = m_color;
    soft.setAlphaF(0.18);
    p->fillPath(fill, soft);
    QPen pen(m_color, line);
    pen.setJoinStyle(Qt::RoundJoin);
    pen.setCapStyle(Qt::RoundCap);
    p->strokePath(stroke, pen);
}
