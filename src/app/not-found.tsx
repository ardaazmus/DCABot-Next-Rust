import Link from "next/link";
import { Terminal, Home } from "lucide-react";
import { Button } from "@/components/ui/button";

export default function NotFound() {
  return (
    <div className="flex min-h-screen w-full flex-col items-center justify-center bg-[#0B0E14] p-4 text-slate-200">
      <div className="w-full max-w-md text-center">
        <div className="mx-auto mb-4 flex h-14 w-14 items-center justify-center rounded-2xl border border-slate-700 bg-surface text-slate-400">
          <Terminal className="h-7 w-7 text-emerald-400" />
        </div>
        <h1 className="text-4xl font-black tracking-tight text-slate-100">404</h1>
        <p className="mt-2 text-sm text-slate-400">Aradığınız terminal görünümü veya sayfa mevcut değil.</p>
        <div className="mt-6 flex justify-center">
          <Button asChild className="bg-emerald-600 font-bold text-white hover:bg-emerald-500">
            <Link href="/">
              <Home className="mr-2 h-4 w-4" />
              Kokpite Dön
            </Link>
          </Button>
        </div>
      </div>
    </div>
  );
}
