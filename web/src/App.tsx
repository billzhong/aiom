import { I18nToggle } from "@/components/i18n-toggle"
import { LocationFilter } from "@/components/location-filter"
import { ModeToggle } from "@/components/mode-toggle"
import { MonitorProvider } from "@/components/monitor-provider"
import { NodeList } from "@/components/node-list"
import { StatusCards } from "@/components/status-cards"
import { Toaster } from "@/components/ui/sonner"

export default function App() {
  return (
    // At least a screen tall, so that with few nodes the footer still ends the page.
    <div className="mx-3 flex min-h-svh max-w-7xl flex-col xl:mx-auto">
      {/* Inset by px-2 like the content below; main's top padding spaces the two. */}
      <header className="flex items-center justify-between px-2 pt-4">
        <h1 className="flex items-center gap-3">
          <span className="text-2xl font-bold tracking-tight text-primary sm:text-3xl">AIO</span>{" "}
          {/* A span, as a heading cannot hold the <div> of a Separator. */}
          <span className="h-6 w-px bg-border" aria-hidden />
          {/* Part of the name, like the page title, so not translated. */}
          <span className="text-lg font-light text-muted-foreground sm:text-xl">Monitor</span>
        </h1>
        <div className="flex items-center gap-2">
          <I18nToggle />
          <ModeToggle />
        </div>
      </header>
      <main className="@container/main flex flex-1 flex-col gap-4 py-4 md:gap-6">
        <MonitorProvider>
          <LocationFilter />
          <StatusCards />
          <NodeList />
        </MonitorProvider>
      </main>
      <footer className="px-2 py-4 text-center">
        <p className="text-sm text-muted-foreground">
          Powered by{" "}
          <a
            href="https://github.com/billzhong/aiom"
            target="_blank"
            rel="noreferrer"
            className="underline-offset-4 hover:text-foreground hover:underline"
          >
            AIO Monitor
          </a>
        </p>
      </footer>
      <Toaster position="top-center" />
    </div>
  )
}
