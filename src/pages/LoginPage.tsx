import { BadgeCheck, LogIn } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

interface LoginPageProps {
  error: string | null;
  onLogin: () => void;
}

export function LoginPage({ error, onLogin }: LoginPageProps) {
  return (
    <div className="flex min-h-screen items-center justify-center bg-gradient-to-br from-slate-50 to-slate-100 p-6">
      <Card className="w-full max-w-md">
        <CardHeader className="text-center">
          <div className="mx-auto mb-4 flex h-14 w-14 items-center justify-center rounded-2xl bg-primary text-primary-foreground">
            <BadgeCheck className="h-7 w-7" />
          </div>
          <CardTitle className="text-2xl">ZD Switch</CardTitle>
          <CardDescription>
            登录后即可自动配置 Codex、Claude Code、Gemini CLI 的代理地址和 Token
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {error && (
            <div className="rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm text-destructive">
              {error}
            </div>
          )}
          <Button className="w-full" size="lg" onClick={onLogin}>
            <LogIn />
            OAuth 授权登录
          </Button>
        </CardContent>
      </Card>
    </div>
  );
}
