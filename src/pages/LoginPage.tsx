import { LogIn } from "lucide-react";
import { AppIcon } from "@/components/ui/app-icon";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

interface LoginPageProps {
  onLogin: () => void;
}

export function LoginPage({ onLogin }: LoginPageProps) {
  return (
    <div className="app-canvas flex min-h-screen items-center justify-center p-6">
      <Card className="w-full max-w-md shadow-lg">
        <CardHeader className="text-center">
          <AppIcon size="md" className="mx-auto mb-4 shadow-sm" />
          <CardTitle className="text-2xl">ZWitch</CardTitle>
          <CardDescription>
            登录后即可自动配置 Codex、Claude Code、Gemini CLI 的代理地址和 Token
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Button className="w-full" size="lg" onClick={onLogin}>
            <LogIn />
            OAuth 授权登录
          </Button>
        </CardContent>
      </Card>
    </div>
  );
}
