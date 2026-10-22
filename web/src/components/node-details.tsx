import {
  Boxes,
  Cable,
  ChevronsLeftRightEllipsis,
  Cpu,
  EthernetPort,
  HardDrive,
  HardDriveDownload,
  HardDriveUpload,
  Layers,
  MemoryStick,
  Microchip,
  Power,
  Rss,
  Variable,
} from "lucide-react"
import type { ReactNode } from "react"
import { useTranslation } from "react-i18next"

import { CountryFlag } from "@/components/country-flag"
import { OsLogo } from "@/components/os-logo"
import { countryName } from "@/lib/countries"
import { formatBytes, formatTimestamp } from "@/lib/format"
import type { NodeData } from "@/lib/types"

function Detail({
  icon,
  label,
  children,
}: {
  icon: ReactNode
  label: string
  children: ReactNode
}) {
  return (
    <div className="flex items-center gap-2">
      <div className="flex size-6 shrink-0 items-center justify-center rounded-full bg-border">
        {icon}
      </div>
      <div>
        <p className="text-xs text-muted-foreground">{label}</p>
        {children}
      </div>
    </div>
  )
}

function Capacity({ bytes }: { bytes: number }) {
  return (
    <p className="flex items-center gap-0.5 text-xs">
      <Boxes className="size-3 text-muted-foreground" aria-hidden />
      {formatBytes(bytes)}
    </p>
  )
}

function Connections({ protocol, count }: { protocol: string; count: number }) {
  return (
    <span className="flex items-center gap-1">
      <span className="flex items-center gap-0.5 font-mono text-xs text-muted-foreground">
        <Cable className="size-3" aria-hidden />
        {protocol}
      </span>
      <span className="font-mono">{count}</span>
    </span>
  )
}

/** Whether `v12n` names an actual virtualization technology. */
function isVirtualized(v12n: string) {
  return !!v12n && v12n.toLowerCase() !== "none"
}

/** Hardware, software and traffic facts of a node. */
export function NodeDetails({ node }: { node: NodeData }) {
  const { t, i18n } = useTranslation()
  const { host, stats } = node
  const code = node.geoip
  const location = countryName(code, i18n.resolvedLanguage) ?? t("unknown")

  return (
    <div className="grid grid-cols-1 content-start gap-3 md:w-1/3">
      <Detail icon={<Cpu className="size-4" />} label={t("cpu")}>
        <p className="text-sm">{host.cpu}</p>
        <p className="text-xs">
          {host.cpu_count} {t("cores", { count: host.cpu_count })}
        </p>
      </Detail>
      <div className="grid grid-cols-1 gap-2 lg:grid-cols-2">
        <Detail icon={<OsLogo os={host.os} />} label={t("os")}>
          <p className="text-sm">
            {host.os} {host.os_version}
          </p>
          <p className="text-xs">
            {host.cpu_arch}
            {isVirtualized(host.v12n) && ` / ${host.v12n}`}
          </p>
        </Detail>
        <Detail
          icon={<CountryFlag code={code} className="size-4 grayscale" />}
          label={t("location")}
        >
          <p className="text-sm">{location}</p>
        </Detail>
      </div>
      <div className="grid grid-cols-1 gap-2 lg:grid-cols-2">
        <Detail icon={<MemoryStick className="size-4" />} label={t("ram")}>
          <p className="text-sm">{formatBytes(stats.used_memory)}</p>
          <Capacity bytes={host.total_memory} />
        </Detail>
        <Detail icon={<Layers className="size-4" />} label={t("swap")}>
          <p className="text-sm">{formatBytes(stats.used_swap)}</p>
          <Capacity bytes={host.total_swap} />
        </Detail>
      </div>
      <div className="grid grid-cols-1 gap-2 lg:grid-cols-2">
        <Detail icon={<HardDrive className="size-4" />} label={t("disk")}>
          <p className="text-sm">{formatBytes(stats.used_disk)}</p>
          <Capacity bytes={host.total_disk} />
        </Detail>
        <Detail icon={<ChevronsLeftRightEllipsis className="size-4" />} label={t("transfer")}>
          <p className="flex flex-col text-sm">
            <span className="flex items-center gap-0.5">
              <HardDriveUpload className="size-3 text-muted-foreground" aria-label={t("upload")} />
              {formatBytes(stats.network_out)}
            </span>
            <span className="flex items-center gap-0.5">
              <HardDriveDownload
                className="size-3 text-muted-foreground"
                aria-label={t("download")}
              />
              {formatBytes(stats.network_in)}
            </span>
          </p>
        </Detail>
      </div>
      <div className="grid grid-cols-1 gap-2 lg:grid-cols-2">
        <Detail icon={<Microchip className="size-4" />} label={t("processes")}>
          <p className="text-sm">{stats.process_count}</p>
          <p className="flex items-center gap-0.5 text-xs">
            <Variable className="size-3 text-muted-foreground" aria-hidden />v{host.agent}
          </p>
        </Detail>
        <Detail icon={<EthernetPort className="size-4" />} label={t("connections")}>
          <p className="flex flex-col text-sm">
            <Connections protocol="TCP" count={stats.tcp_count} />
            <Connections protocol="UDP" count={stats.udp_count} />
          </p>
        </Detail>
      </div>
      <div className="grid grid-cols-1 gap-2 lg:grid-cols-2">
        <Detail icon={<Power className="size-4" />} label={t("boot-time")}>
          <p className="text-sm">{formatTimestamp(stats.boot_time)}</p>
        </Detail>
        <Detail icon={<Rss className="size-4" />} label={t("last-update")}>
          <p className="text-sm">{formatTimestamp(node.timestamp)}</p>
        </Detail>
      </div>
    </div>
  )
}
