import React from "react";
import { Dialog, DialogContent, DialogTitle, DialogTrigger } from "./ui/dialog";
import { VisuallyHidden } from "./ui/visually-hidden";
import { About } from "./About";
import { translateUI, uiI18n } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';


interface LogoProps {
    isCollapsed: boolean;
}

const Logo = React.forwardRef<HTMLButtonElement, LogoProps>(({ isCollapsed }, ref) => {
  useUiTranslation();
  return (
    <Dialog aria-describedby={undefined}>
      {isCollapsed ? (
        <DialogTrigger asChild>
          <button ref={ref} aria-label={translateUI("About HuiTrace")} className="flex items-center justify-start cursor-pointer rounded-xl border-none bg-transparent p-1 transition-colors hover:bg-muted">
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img src="/huitrace-icon-yin-wave.svg" alt="HuiTrace" width={30} height={30} className="block shrink-0" />
          </button>
        </DialogTrigger>
      ) : (
        <DialogTrigger asChild>
          <button ref={ref} aria-label={translateUI("About HuiTrace")} className="group -mx-1 flex items-center gap-2 rounded-lg px-1.5 py-1 transition-colors hover:bg-muted">
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img src="/huitrace-icon-yin-wave-small.svg" alt="" width={24} height={24} className="block shrink-0" />
            <span className="font-heading text-[15px] font-semibold leading-none tracking-tight text-foreground">
              {uiI18n.language.startsWith('zh') ? '会迹' : 'HuiTrace'}
            </span>
          </button>
        </DialogTrigger>
      )}
      <DialogContent>
        <VisuallyHidden>
          <DialogTitle>{translateUI("About HuiTrace")}</DialogTitle>
        </VisuallyHidden>
        <About />
      </DialogContent>
    </Dialog>
  );
});

Logo.displayName = "Logo";

export default Logo;
