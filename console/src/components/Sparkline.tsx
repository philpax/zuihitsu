/// A minimal trend line in the Japandi manner — a thin stroke, a single emphasized last point, a
/// hairline baseline, and nothing else. Hand-drawn SVG rather than a charting library, so it carries
/// no foreign theme; the views set the stroke from the palette (sage when a metric is healthy, clay
/// when it is not). Values map left-to-right, oldest to newest.
export function Sparkline({
  values,
  domainMax,
  stroke = "var(--color-sage)",
  width = 132,
  height = 30,
}: {
  values: number[];
  domainMax?: number;
  stroke?: string;
  width?: number;
  height?: number;
}) {
  if (values.length === 0) return <svg width={width} height={height} />;

  const pad = 3;
  const max = domainMax ?? Math.max(...values);
  const span = max > 0 ? max : 1;
  const x = (index: number) =>
    values.length === 1 ? width / 2 : pad + (index / (values.length - 1)) * (width - 2 * pad);
  const y = (value: number) => height - pad - (value / span) * (height - 2 * pad);

  const points = values.map((value, index) => `${x(index).toFixed(1)},${y(value).toFixed(1)}`);
  const lastIndex = values.length - 1;

  return (
    <svg width={width} height={height} className="overflow-visible">
      <line
        x1={0}
        y1={height - pad}
        x2={width}
        y2={height - pad}
        stroke="var(--color-line)"
        strokeWidth={1}
      />
      {values.length > 1 && (
        <polyline points={points.join(" ")} fill="none" stroke={stroke} strokeWidth={1.25} />
      )}
      <circle cx={x(lastIndex)} cy={y(values[lastIndex])} r={2} fill={stroke} />
    </svg>
  );
}
