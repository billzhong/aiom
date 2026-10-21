import type { ComponentProps } from "react"
import { useTranslation } from "react-i18next"
import { Area, AreaChart, CartesianGrid, Line, LineChart, XAxis, YAxis } from "recharts"

import {
  type ChartConfig,
  ChartContainer,
  ChartTooltip,
  ChartTooltipContent,
} from "@/components/ui/chart"
import { formatBitRate, formatPercent } from "@/lib/format"
import type { ChartPoint } from "@/lib/types"

function ChartPanel({
  title,
  config,
  children,
}: {
  title: string
  config: ChartConfig
  children: ComponentProps<typeof ChartContainer>["children"]
}) {
  return (
    <div className="flex flex-col">
      <p className="text-center">{title}</p>
      <ChartContainer config={config}>{children}</ChartContainer>
    </div>
  )
}

/** One row of a chart tooltip. */
function ChartTooltipRow({ name, value }: { name: string; value: string }) {
  return (
    <div className="flex min-w-[130px] items-center text-xs text-muted-foreground">
      {name}
      <div className="ml-auto flex items-baseline gap-0.5 font-mono font-medium text-foreground tabular-nums">
        {value}
      </div>
    </div>
  )
}

function TimeAxis() {
  return <XAxis dataKey="time" tickLine={false} axisLine={false} tickMargin={10} minTickGap={30} />
}

function PercentAxis() {
  return (
    <YAxis
      tickLine={false}
      axisLine={false}
      tickMargin={10}
      domain={[0, 100]}
      tickFormatter={(value) => `${value}%`}
    />
  )
}

function lines(keys: readonly (keyof ChartPoint)[]) {
  return keys.map((key) => (
    <Line
      key={key}
      dataKey={key}
      type="monotone"
      isAnimationActive={false}
      stroke={`var(--color-${key})`}
      strokeWidth={2}
      dot={false}
    />
  ))
}

/** History charts of CPU, memory, network speed and connections. */
export function NodeCharts({ history }: { history: ChartPoint[] }) {
  const { t } = useTranslation()
  const tooltip = (format: (value: unknown) => string) => (
    <ChartTooltip
      cursor={false}
      content={
        <ChartTooltipContent
          hideLabel
          formatter={(value, name) => (
            <ChartTooltipRow name={t(String(name))} value={format(value)} />
          )}
        />
      }
    />
  )
  const percentTooltip = tooltip((value) => formatPercent(Number(value)))

  return (
    <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
      <ChartPanel title={t("cpu")} config={{ cpu: { label: t("cpu"), color: "var(--chart-3)" } }}>
        <LineChart accessibilityLayer data={history} margin={{ left: -10, right: 10 }}>
          <CartesianGrid vertical={false} />
          {TimeAxis()}
          {PercentAxis()}
          {percentTooltip}
          {lines(["cpu"])}
        </LineChart>
      </ChartPanel>
      <ChartPanel
        title={t("ram")}
        config={{
          ram: { label: t("ram"), color: "var(--chart-1)" },
          swap: { label: t("swap"), color: "var(--chart-2)" },
        }}
      >
        <AreaChart accessibilityLayer data={history} margin={{ left: -10, right: 10 }}>
          <CartesianGrid vertical={false} />
          {TimeAxis()}
          {PercentAxis()}
          {percentTooltip}
          {(["ram", "swap"] as const).map((key) => (
            <Area
              key={key}
              dataKey={key}
              type="monotone"
              fill={`var(--color-${key})`}
              fillOpacity={0.4}
              stroke={`var(--color-${key})`}
              strokeWidth={2}
              isAnimationActive={false}
            />
          ))}
        </AreaChart>
      </ChartPanel>
      <ChartPanel
        title={t("network")}
        config={{
          download: { label: t("download"), color: "var(--chart-1)" },
          upload: { label: t("upload"), color: "var(--chart-2)" },
        }}
      >
        <LineChart accessibilityLayer data={history} margin={{ left: 10, right: 10 }}>
          <CartesianGrid vertical={false} />
          {TimeAxis()}
          <YAxis
            tickLine={false}
            axisLine={false}
            tickMargin={10}
            padding={{ top: 15 }}
            tickFormatter={(value) => formatBitRate(value)}
          />
          {tooltip((value) => formatBitRate(Number(value)))}
          {lines(["download", "upload"])}
        </LineChart>
      </ChartPanel>
      <ChartPanel
        title={t("connections")}
        config={{
          tcp: { label: "TCP", color: "var(--chart-1)" },
          udp: { label: "UDP", color: "var(--chart-2)" },
        }}
      >
        <LineChart accessibilityLayer data={history} margin={{ left: 10, right: 10 }}>
          <CartesianGrid vertical={false} />
          {TimeAxis()}
          <YAxis tickLine={false} axisLine={false} tickMargin={10} padding={{ top: 5 }} />
          <ChartTooltip cursor={false} content={<ChartTooltipContent hideLabel hideIndicator />} />
          {lines(["tcp", "udp"])}
        </LineChart>
      </ChartPanel>
    </div>
  )
}
