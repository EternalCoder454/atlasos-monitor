#include "coregrid.h"

#include <QFontMetricsF>
#include <QPainter>

#include <algorithm>
#include <cmath>

namespace {
// The bar under the figures, and the gap between them, in pixels at 1x.
constexpr double BarHeight = 6;
constexpr double BarGap = 4;
} // namespace

CoreGrid::CoreGrid(QQuickItem *parent)
    : QQuickPaintedItem(parent)
{
    // Restyling repaints too; a new count or font also moves the cells.
    connect(this, &CoreGrid::styleChanged, this, [this] {
        relayout();
        update();
    });
}

void CoreGrid::setValues(const QList<qreal> &values)
{
    if (values == m_values) {
        return;
    }
    const bool recount = values.size() != m_values.size();
    m_values = values;
    if (recount) {
        m_labels.clear();
        relayout();
    }
    Q_EMIT valuesChanged();
    update();
}

void CoreGrid::setFont(const QFont &font)
{
    if (font == m_font) {
        return;
    }
    m_font = font;
    m_labels.clear();
    Q_EMIT styleChanged();
}

void CoreGrid::setLabelFormat(const QString &format)
{
    if (format == m_labelFormat) {
        return;
    }
    m_labelFormat = format;
    m_labels.clear();
    Q_EMIT styleChanged();
}

void CoreGrid::setMinimumCellWidth(qreal w)
{
    if (w == m_minimumCellWidth || w <= 0) {
        return;
    }
    m_minimumCellWidth = w;
    Q_EMIT styleChanged();
}

void CoreGrid::setColumnSpacing(qreal s)
{
    if (s == m_columnSpacing || s < 0) {
        return;
    }
    m_columnSpacing = s;
    Q_EMIT styleChanged();
}

void CoreGrid::setRowSpacing(qreal s)
{
    if (s == m_rowSpacing || s < 0) {
        return;
    }
    m_rowSpacing = s;
    Q_EMIT styleChanged();
}

void CoreGrid::itemChange(ItemChange change, const ItemChangeData &value)
{
    // Laid out for one scale: another screen's would lay them out again on
    // every paint.
    if (change == ItemDevicePixelRatioHasChanged) {
        m_labels.clear();
        update();
    }
    QQuickPaintedItem::itemChange(change, value);
}

void CoreGrid::geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry)
{
    QQuickPaintedItem::geometryChange(newGeometry, oldGeometry);
    if (newGeometry.width() != oldGeometry.width()) {
        relayout();
    }
}

double CoreGrid::cellHeight() const
{
    return std::ceil(QFontMetricsF(m_font).height()) + BarGap + BarHeight;
}

void CoreGrid::relayout()
{
    const double w = width();
    const int n = int(m_values.size());
    // As many cells of the minimum width as fit, but no more than there are.
    const int fit = std::clamp(w > 0 ? int((w + m_columnSpacing) / (m_minimumCellWidth + m_columnSpacing)) : 1, 1, std::max(1, n));
    // Rather a few less that fill every row (8 of 32, not 9): a short last
    // row reads as processors missing.
    int columns = fit;
    for (int c = fit; c * 4 >= fit * 3; --c) {
        if (n % c == 0) {
            columns = c;
            break;
        }
    }
    if (columns != m_columns) {
        m_columns = columns;
        Q_EMIT columnsChanged();
    }
    const int rows = (n + columns - 1) / columns;
    setImplicitHeight(rows > 0 ? rows * cellHeight() + (rows - 1) * m_rowSpacing : 0);
}

void CoreGrid::paint(QPainter *p)
{
    const int n = int(m_values.size());
    if (n == 0 || width() <= 0) {
        return;
    }
    p->setFont(m_font);
    const QFontMetricsF fm(m_font);
    if (m_labels.size() != n || p->transform() != m_labelTransform) {
        m_labelTransform = p->transform();
        m_labels.clear();
        m_labels.reserve(n);
        for (int i = 0; i < n; ++i) {
            QStaticText t(m_labelFormat.arg(i));
            t.setTextFormat(Qt::PlainText);
            t.prepare(m_labelTransform, m_font);
            m_labels.append(t);
        }
    }

    const double cellW = (width() - m_columnSpacing * (m_columns - 1)) / m_columns;
    const double cellH = cellHeight();
    const double textH = std::ceil(fm.height());
    const double radius = BarHeight / 2;
    QColor label = m_textColor;
    label.setAlphaF(0.75);
    QColor figure = m_textColor;
    figure.setAlphaF(0.55);
    QColor track = m_textColor;
    track.setAlphaF(0.14);

    p->setRenderHint(QPainter::Antialiasing, true);
    for (int i = 0; i < n; ++i) {
        const int row = i / m_columns;
        const int col = i % m_columns;
        const double x = m_mirrored ? width() - (col + 1) * cellW - col * m_columnSpacing : col * (cellW + m_columnSpacing);
        const double y = row * (cellH + m_rowSpacing);
        const double v = m_values[i];
        const bool known = std::isfinite(v);
        const double share = known && m_maximum > 0 ? std::clamp(v / m_maximum, 0.0, 1.0) : 0;

        // The name at the start, the figure at the end.
        const QString text = known ? QString::number(std::lround(v)) + QLatin1Char('%') : QStringLiteral("–");
        const double textW = fm.horizontalAdvance(text);
        const QStaticText &name = m_labels[i];
        const double nameW = std::max(0.0, std::min<double>(name.size().width(), cellW - textW - 4));
        p->setPen(label);
        p->save();
        p->setClipRect(QRectF(m_mirrored ? x + cellW - nameW : x, y, nameW, textH));
        p->drawStaticText(QPointF(m_mirrored ? x + cellW - name.size().width() : x, y), name);
        p->restore();
        p->setPen(figure);
        p->drawText(QPointF(m_mirrored ? x : x + cellW - textW, y + fm.ascent()), text);

        // A pill: the track, and the load filling it from the start, at
        // least a circle so a light load still shows.
        const QRectF bar(x, y + textH + BarGap, cellW, BarHeight);
        p->setPen(Qt::NoPen);
        p->setBrush(track);
        p->drawRoundedRect(bar, radius, radius);
        if (share > 0) {
            const double fillW = std::max(BarHeight, cellW * share);
            p->setBrush(m_color);
            p->drawRoundedRect(QRectF(m_mirrored ? bar.right() - fillW : bar.left(), bar.top(), fillW, BarHeight), radius, radius);
        }
    }
}
