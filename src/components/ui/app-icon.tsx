import appIcon from "@/assets/app-icon.png";
import { cn } from "@/lib/utils";

const SIZE_CLASS = {
  sm: "h-9 w-9",
  md: "h-14 w-14",
  lg: "h-20 w-20",
} as const;

interface AppIconProps {
  size?: keyof typeof SIZE_CLASS;
  className?: string;
}

export function AppIcon({ size = "sm", className }: AppIconProps) {
  return (
    <img
      src={appIcon}
      alt="ZWitch"
      className={cn(SIZE_CLASS[size], "shrink-0 rounded-xl object-cover", className)}
    />
  );
}
