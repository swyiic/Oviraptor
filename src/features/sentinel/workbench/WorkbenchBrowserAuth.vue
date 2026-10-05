<script setup lang="ts">
import { Check, ChevronDown, KeyRound, LogIn, Plus, RefreshCw, ShieldCheck, Trash2 } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { useBrowserAuthSessions } from "./useBrowserAuthSessions";

type AuthControls = ReturnType<typeof useBrowserAuthSessions>;
const props = defineProps<{
  form: { projectId: number; authSessionIds: string[]; authLoginUrl: string; authSessionName: string };
  controls: AuthControls;
}>();
const { tr } = useI18n();
const { authBusy, authSessions, showAuthSessionPicker, firstUrl, authStatusLabel,
  shortTime, onAuthSessionToggle, openLogin, finishLogin, validateLogin, removeLogin } = props.controls;
</script>

<template>
  <section class="browser-auth-center span-two" :class="{ expanded: showAuthSessionPicker }">
    <header>
      <div class="browser-auth-title"><LogIn :size="17" /><div><strong>{{ tr("登录身份（可选）", "Login identities (optional)") }}</strong><small>{{ form.authSessionIds.length ? tr(`当前任务已选择 ${form.authSessionIds.length} 个独立身份；两个以上将生成 IDOR 权限差异矩阵。`, `${form.authSessionIds.length} isolated identities selected; two or more produce an IDOR differential matrix.`) : tr("默认按匿名身份扫描；需要验证码、SSO 或多账户 IDOR 对比时再展开选择。", "Anonymous scanning is the default. Expand only for CAPTCHA, SSO, or multi-account IDOR comparison.") }}</small></div></div>
      <div class="auth-picker-actions"><span class="auth-global-state" :class="{ valid: form.authSessionIds.length > 0 }"><i></i>{{ form.authSessionIds.length ? tr(`已选择 ${form.authSessionIds.length} 个`, `${form.authSessionIds.length} selected`) : tr(`${authSessions.filter((session) => session.status === 'valid').length} 个可用`, `${authSessions.filter((session) => session.status === 'valid').length} available`) }}</span><button class="button ghost auth-picker-toggle" type="button" @click="showAuthSessionPicker = !showAuthSessionPicker">{{ showAuthSessionPicker ? tr("收起", "Collapse") : tr("选择登录身份", "Choose identities") }}<ChevronDown :size="14" :class="{ rotated: showAuthSessionPicker }" /></button></div>
    </header>
    <div v-if="showAuthSessionPicker" class="auth-session-create">
      <label class="field"><span>{{ tr("登录入口", "Login URL") }}</span><input v-model="form.authLoginUrl" :placeholder="firstUrl() || 'https://app.example.com/login'" /></label>
      <label class="field"><span>{{ tr("身份名称", "Identity name") }}</span><input v-model="form.authSessionName" :placeholder="tr('管理员账户 / 普通账户 A', 'Administrator / normal account A')" /></label>
      <button class="button primary auth-login-button" type="button" :disabled="Boolean(authBusy) || !form.projectId" @click="openLogin()"><Plus :size="15" />{{ authBusy === 'new' ? tr("正在打开…", "Opening…") : tr("新增独立身份", "Add isolated identity") }}</button>
    </div>
    <div v-if="showAuthSessionPicker && authSessions.length" class="auth-session-list">
      <p class="auth-identity-hint"><ShieldCheck :size="13" />{{ tr("每次新增都会打开独立 WebView 数据容器，Cookie 与 Storage 不会继承其他账户；后端还会拒绝认证材料相同或没有共同作用域的身份组合。", "Each identity opens an isolated WebView data store. The backend rejects duplicate authentication material or identities without a shared scope.") }}</p>
      <article v-for="session in authSessions" :key="session.id" class="auth-session-card" :class="[{ selected: form.authSessionIds.includes(session.id) }, `status-${session.status}`]">
        <label class="auth-session-select"><input type="checkbox" :checked="form.authSessionIds.includes(session.id)" :disabled="session.status !== 'valid'" @change="onAuthSessionToggle(session.id, $event)" /><span class="auth-status-dot"></span><span><strong>{{ session.name }}</strong><small>{{ authStatusLabel(session.status) }} · {{ shortTime(session.lastValidatedAt || session.updatedAt) }}</small></span></label>
        <div class="auth-session-metrics"><span><b>{{ session.cookieCount }}</b> Cookie</span><span><b>{{ session.headerCount }}</b> {{ tr("认证头", "auth headers") }}</span><span><b>{{ session.storageCount }}</b> Storage</span><span><b>{{ session.capturedRequestCount }}</b> {{ tr("请求", "requests") }}</span></div>
        <p class="auth-session-scope"><span>{{ tr("作用域", "Scope") }}</span>{{ session.scopeHosts.slice(0, 5).join(" · ") || tr("等待登录后生成", "Generated after login") }}</p>
        <p v-if="session.lastError" class="auth-session-error">{{ session.lastError }}</p>
        <div class="auth-session-actions">
          <button v-if="session.status === 'capturing'" class="button primary" type="button" :disabled="Boolean(authBusy)" @click="finishLogin(session)"><Check :size="14" />{{ tr("我已登录，完成捕获", "I'm signed in — finish") }}</button>
          <button v-else class="button ghost" type="button" :disabled="Boolean(authBusy)" @click="openLogin(session)"><RefreshCw :size="13" />{{ tr("重新登录", "Sign in again") }}</button>
          <button v-if="session.status !== 'capturing'" class="button ghost" type="button" :disabled="Boolean(authBusy)" @click="validateLogin(session)"><ShieldCheck :size="13" />{{ tr("校验", "Validate") }}</button>
          <button class="icon-button danger" type="button" :disabled="Boolean(authBusy)" :title="tr('删除会话', 'Delete session')" @click="removeLogin(session)"><Trash2 :size="14" /></button>
        </div>
      </article>
    </div>
    <div v-if="showAuthSessionPicker && !authSessions.length" class="auth-session-empty"><KeyRound :size="18" /><span>{{ tr("适合验证码、扫码、SSO 和多步骤登录。每个账户分别点击“新增独立身份”，完成登录并访问一个后台功能后再保存。", "For CAPTCHA, QR, SSO, and multi-step login, add every account as a separate isolated identity and visit one authenticated feature before saving.") }}</span></div>
    <footer v-if="showAuthSessionPicker"><ShieldCheck :size="13" /><span>{{ tr("会话仅保存在本机 SQLite 和权限 600 的任务文件中；自动头由浏览器重建。单个 401/403 不熄灯，明确跳回登录页才判失效。", "Session data stays in local SQLite and mode-600 task files. Browser-managed headers are regenerated. One 401/403 does not invalidate the session; a clear redirect to login does.") }}</span></footer>
  </section>
</template>
