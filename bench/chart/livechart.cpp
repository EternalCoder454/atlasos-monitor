#include "livechart.h"

#include <QElapsedTimer>
#include <QFontMetricsF>
#include <QPainter>

#include <algorithm>
#include <cmath>

qint64 LiveChart::s_paintNanos = 0;
qint64 LiveChart::s_paints = 0;

void Series::push(double v)
{
    m_buf[m_head] = v;
    m_head = (m_head + 1) % Capacity;
    m_len = std::min(m_len + 1, Capacity);
    m_values.resize(m_len);
    copyInto(m_values.data());
    Q_EMIT changed();
}

int Series::copyInto(double *out) const
{
    const int start = (m_head - m_len + Capacity) % Capacity;
    for (int i = 0; i < m_len; ++i) {
        out[i] = m_buf[(start + i) % Capacity];
    }
    return m_len;
}

double Series::max() const
{
    double m = 0;
    for (int i = 0; i < m_len; ++i) {
        m = std::max(m, m_buf[i]);
    }
    return m;
}

LiveChart::LiveChart(QQuickItem *parent)
    : QQuickPaintedItem(parent)
{
    setAntialiasing(true);
    // Any change to how the chart looks repaints it now, not at the next tick.
    for (auto signal : {&LiveChart::labelChanged, &LiveChart::colorChanged, &LiveChart::styleChanged}) {
        connect(this, signal, this, [this] { update(); });
    }
}

void LiveChart::setSeries(Series *s)
{
    if (m_series == s) {
        return;
    }
    if (m_series) {
        disconnect(m_series, nullptr, this, nullptr);
    }
    m_series = s;
    if (s) {
        connect(s, &Series::changed, this, [this] { update(); });
    }
    update();
    Q_EMIT seriesChanged();
}

void LiveChart::setValues(const QList<qreal> &v)
{
    m_values = v;
    update();
    Q_EMIT valuesChanged();
}

// Sets the static text only when it changed, so a steady caption is not
// re-laid out, then draws it.
void LiveChart::drawText(QPainter *p, QStaticText &t, QString &shown, const QString &text, QPointF at, double alpha)
{
    if (text != shown) {
        shown = text;
        t.setText(text);
        t.setTextFormat(Qt::PlainText);
        t.prepare(p->transform(), p->font());
    }
    QColor c = m_textColor;
    c.setAlphaF(alpha);
    p->setPen(c);
    p->drawStaticText(at, t);
}

// Benchmark-only switches: CHART_SKIP=grid,fill,line,border,text
static bool skip(const char *part)
{
    static const QByteArray s = qgetenv("CHART_SKIP");
    return s.split(',').contains(part);
}

// The middle of the device pixel v falls in, so a 1-device-pixel line is
// one sharp pixel instead of two half-strength ones (also at 1.5x).
static double crisp(double v, double dpr)
{
    return (std::floor(v * dpr) + 0.5) / dpr;
}

void LiveChart::paint(QPainter *p)
{
    QElapsedTimer timer;
    timer.start();

    const double dpr = p->device()->devicePixelRatioF();
    const double w = width();
    const double h = height();
    if (w <= 0 || h <= 0) {
        return;
    }
    int n = 0;
    if (m_series) {
        n = m_series->copyInto(m_scratch.data());
    } else {
        n = std::min<int>(m_values.size(), Series::Capacity);
        std::copy(m_values.cend() - n, m_values.cend(), m_scratch.begin());
    }
    if (n == 0) {
        m_scratch[0] = 0; // no stale sample from an earlier series sets the scale
    }
    const double cur = n > 0 ? m_scratch[n - 1] : 0;

    const QFontMetricsF fm(p->font());
    const double lineH = std::ceil(fm.height());
    const double band = m_captions ? lineH + 4 : 0;
    const double top = band;
    const double plotH = std::max(0.0, h - 2 * band);

    double scale = 100;
    if (!m_percent) {
        scale = std::max(1.0, *std::max_element(m_scratch.begin(), m_scratch.begin() + std::max(n, 1)) * 1.25);
    }

    QColor grid = m_textColor;
    if (plotH >= 2) {
        // Grid: rows divide the plot evenly, columns repeat the row height
        // leftwards from the newest sample, so every cell is square.
        const int rows = std::clamp(int(std::round(plotH / 36)), 2, 10);
        const double cell = plotH / rows;
        // Grid and border are one device pixel wide, filled as rectangles:
        // translucent cosmetic lines go through Qt's per-pixel line path,
        // rectangles through its span blender (2-3x cheaper on a big chart).
        static const bool gridLines = qgetenv("CHART_GRID") == "lines";
        const double px = 1 / dpr;
        auto snap = [dpr](double v) { return std::floor(v * dpr) / dpr; };
        if (!skip("grid")) {
            grid.setAlphaF(0.08);
            p->setRenderHint(QPainter::Antialiasing, false);
            p->setPen(QPen(grid, 0)); // cosmetic: one device pixel
            for (int i = 1; i < rows; ++i) {
                if (gridLines) {
                    const double y = crisp(top + cell * i, dpr);
                    p->drawLine(QPointF(0, y), QPointF(w, y));
                } else {
                    p->fillRect(QRectF(0, snap(top + cell * i), w, px), grid);
                }
            }
            for (int k = 1;; ++k) {
                const double x = w - cell * k;
                if (x <= 1) {
                    break;
                }
                if (gridLines) {
                    p->drawLine(QPointF(crisp(x, dpr), top), QPointF(crisp(x, dpr), top + plotH));
                } else {
                    p->fillRect(QRectF(snap(x), top, px, plotH), grid);
                }
            }
        }

        if (n >= 2) {
            const double bottom = top + plotH;
            const double dx = w / (Series::Capacity - 1);
            QPointF pts[Series::Capacity + 2];
            for (int i = 0; i < n; ++i) {
                const double r = std::clamp(m_scratch[i] / scale, 0.0, 1.0);
                pts[i + 1] = QPointF(w - dx * (n - 1 - i), bottom - r * plotH);
            }
            pts[0] = QPointF(pts[1].x(), bottom);
            pts[n + 1] = QPointF(pts[n].x(), bottom);

            static const QByteArray lineMode = qgetenv("CHART_LINE");
            static const bool fillAA = qgetenv("CHART_FILLAA") == "1";
            p->save();
            static const bool noClip = qgetenv("CHART_NOCLIP") == "1";
            if (!noClip) p->setClipRect(QRectF(0, top, w, plotH));
            QColor fill = m_color;
            fill.setAlphaF(0.22);
            p->setPen(Qt::NoPen);
            p->setBrush(fill);
            p->setRenderHint(QPainter::Antialiasing, fillAA);
            if (!skip("fill")) p->drawPolygon(pts, n + 2);
            p->setRenderHint(QPainter::Antialiasing, true);
            p->setBrush(Qt::NoBrush);
            if (skip("line")) {
            } else if (lineMode == "miter" || lineMode == "bevel" || lineMode == "round") {
                const Qt::PenJoinStyle join = lineMode == "miter" ? Qt::MiterJoin : lineMode == "bevel" ? Qt::BevelJoin : Qt::RoundJoin;
                const Qt::PenCapStyle cap = lineMode == "miter" || lineMode == "bevel" ? Qt::FlatCap : Qt::RoundCap;
                p->setPen(QPen(m_color, 1.5, Qt::SolidLine, cap, join));
                p->drawPolyline(pts + 1, n);
            } else if (lineMode == "cosmetic") {
                QPen pen(m_color, 0);
                p->setPen(pen);
                p->drawPolyline(pts + 1, n);
            } else if (lineMode.isEmpty() || lineMode == "segments") {
                // Each segment its own small quad, extended by half the width
                // at both ends so the joins have no gaps.
                const double hw = 0.75;
                p->setBrush(m_color);
                const QPointF *v = pts + 1;
                for (int i = 0; i + 1 < n; ++i) {
                    const QPointF d = v[i + 1] - v[i];
                    const double l = std::hypot(d.x(), d.y());
                    if (l <= 0) continue;
                    const QPointF u = d / l * hw;
                    const QPointF nn(-u.y(), u.x());
                    const QPointF a = v[i] - u, b = v[i + 1] + u;
                    const QPointF quad[4] = {a + nn, b + nn, b - nn, a - nn};
                    p->drawConvexPolygon(quad, 4);
                }
            } else if (lineMode == "cosmetic3") {
                // Three passes of the fast 1-device-pixel line, offset by a
                // device pixel right and down: about 2 px in every direction.
                QPen pen(m_color, 0);
                p->setPen(pen);
                const double px = 1 / p->device()->devicePixelRatioF();
                p->drawPolyline(pts + 1, n);
                p->translate(px, 0);
                p->drawPolyline(pts + 1, n);
                p->translate(-px, px);
                p->drawPolyline(pts + 1, n);
                p->translate(0, -px);
            } else if (lineMode == "outline") {
                // The stroke as one filled ribbon: each vertex pushed out
                // half the width along the bisector of its two segments.
                const double hw = 0.75;
                QPointF ribbon[2 * Series::Capacity];
                const QPointF *v = pts + 1;
                for (int i = 0; i < n; ++i) {
                    QPointF d1 = i > 0 ? v[i] - v[i - 1] : v[i + 1] - v[i];
                    QPointF d2 = i < n - 1 ? v[i + 1] - v[i] : d1;
                    const double l1 = std::hypot(d1.x(), d1.y()), l2 = std::hypot(d2.x(), d2.y());
                    QPointF n1(-d1.y() / l1, d1.x() / l1), n2(-d2.y() / l2, d2.x() / l2);
                    QPointF m = n1 + n2;
                    const double ml = std::hypot(m.x(), m.y());
                    m /= ml;
                    // miter length, capped so sharp spikes don't shoot out
                    const double cosHalf = std::max(0.35, m.x() * n1.x() + m.y() * n1.y());
                    m *= hw / cosHalf;
                    ribbon[i] = v[i] + m;
                    ribbon[2 * n - 1 - i] = v[i] - m;
                }
                p->setBrush(m_color);
                p->drawPolygon(ribbon, 2 * n, Qt::WindingFill);
            }
            p->restore();
        }

        grid.setAlphaF(0.30);
        p->setRenderHint(QPainter::Antialiasing, false);
        if (!skip("border")) {
            const double t = snap(top), b = snap(top + plotH) - px, r = snap(w) - px;
            p->fillRect(QRectF(0, t, w, px), grid);
            p->fillRect(QRectF(0, b, w, px), grid);
            p->fillRect(QRectF(0, t + px, px, b - t - px), grid);
            p->fillRect(QRectF(r, t + px, px, b - t - px), grid);
        }
    }

    if (m_captions && !skip("text")) {
        p->setRenderHint(QPainter::TextAntialiasing, true);
        const QString label = m_label.isEmpty() ? QString() : m_label + QStringLiteral("  ");
        drawText(p, m_labelText, m_shownLabel, label, QPointF(1, 2), 0.66);
        const double lw = label.isEmpty() ? 0 : m_labelText.size().width();
        const QString value = m_percent ? QString::number(cur, 'f', 0) + QLatin1Char('%') : QString::number(cur / 1e6, 'f', 1) + QStringLiteral(" MB/s");
        drawText(p, m_valueText, m_shownValue, value, QPointF(1 + lw, 2), 0.92);
        const QString peak = m_percent ? QStringLiteral("100%") : QStringLiteral("peak ") + QString::number(scale / 1.25e6, 'f', 1) + QStringLiteral(" MB/s");
        const double pw = fm.horizontalAdvance(peak);
        drawText(p, m_peakText, m_shownPeak, peak, QPointF(w - pw - 1, 2), 0.5);
        const double by = h - band + 2;
        drawText(p, m_spanText, m_shownSpan, QStringLiteral("60 seconds"), QPointF(1, by), 0.5);
        drawText(p, m_zeroText, m_shownZero, QStringLiteral("0"), QPointF(w - fm.horizontalAdvance(QStringLiteral("0")) - 1, by), 0.5);
    }

    s_paintNanos += timer.nsecsElapsed();
    ++s_paints;
}
