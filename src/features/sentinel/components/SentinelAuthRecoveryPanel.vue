<script setup lang="ts">
import { computed } from "vue";
import { AlertTriangle, Check, LogIn, RefreshCw, ShieldCheck, X } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { BrowserAuthSession, SentinelScan } from "../types";

const props = defineProps<{
  scan: SentinelScan;
  sessions: BrowserAuthSession[];
  busy: string;
}>();
const emit = defineEmits<{
  reopen: [session: BrowserAuthSession];
  finish: [session: BrowserAuthSession];
  validate: [session: BrowserAuthSession];
  continue: [];
  close: [];
}>();
const { tr } = useI18n();

const ready = computed(
  () => props.sessions.length > 0 && props.sessions.every((session) => session.status === "valid"),
);
const statusLabel = (status: string) =>
  ({
    valid: tr("绿色有效", "Valid"),
    capturing: tr("登录窗口已打开", "Login window open"),
    needs_check: tr("需要校验", "Needs check"),
    invalid: tr("会话已失效", "Session invalid"),
    expired: tr("会话已过期", "Session expired"),
  } as Record<string, string>)[status] || status;
</script>

<template>
  <section class="scan-auth-recovery">
    <header>
      <div class="scan-auth-recovery-title">
        <span class="scan-auth-warning"><AlertTriangle :size="18" /></span>
        <div>
          <h3>{{ tr("原任务需要重新登录", "The original task needs a new login") }}</h3>
          <p>{{ tr(`任务「${scan.taskName || scan.projectName}」仍保留全部历史、证据和累计 Token。只更新它绑定的会话，然后在同一个任务上继续执行。`, `Task "${scan.taskName || scan.projectName}" keeps all its history, evidence and accumulated tokens. Only its bound sessions are updated, then the same task continues.`) }}</p>
        </div>
      </div>
      <button class="icon-button" :title="tr('暂时关闭', 'Close for now')" @click="emit('close')"><X :size="15" /></button>
    </header>

    <div class="scan-auth-session-list">
      <article v-for="session in sessions" :key="session.id" :class="`status-${session.status}`">
        <div class="scan-auth-session-main">
          <span class="auth-status-dot"></span>
          <div>
            <strong>{{ session.name }}</strong>
            <small>{{ statusLabel(session.status) }} · {{ session.entryUrl }}</small>
          </div>
        </div>
        <div class="auth-session-metrics">
          <span><b>{{ session.cookieCount }}</b> Cookie</span>
          <span><b>{{ session.headerCount }}</b> {{ tr("认证头", "auth headers") }}</span>
          <span><b>{{ session.storageCount }}</b> Storage</span>
          <span><b>{{ session.capturedRequestCount }}</b> {{ tr("请求", "requests") }}</span>
        </div>
        <p v-if="session.lastError" class="auth-session-error">{{ session.lastError }}</p>
        <div class="auth-session-actions">
          <button v-if="session.status === 'capturing'" class="button primary compact" :disabled="Boolean(busy)" @click="emit('finish', session)"><Check :size="13" />{{ tr("我已登录，保存会话", "I've logged in, save session") }}</button>
          <button :class="session.status === 'capturing' ? 'button ghost compact' : 'button primary compact'" :disabled="Boolean(busy)" @click="emit('reopen', session)"><LogIn :size="13" />{{ session.status === 'capturing' ? tr('打开登录窗口', 'Open login window') : tr('重新登录', 'Log in again') }}</button>
          <button v-if="session.status !== 'capturing'" class="button ghost compact" :disabled="Boolean(busy)" @click="emit('validate', session)"><ShieldCheck :size="13" />{{ tr("校验当前会话", "Check current session") }}</button>
        </div>
      </article>
    </div>

    <footer>
      <span><ShieldCheck :size="14" />{{ ready ? tr("所有绑定身份均为绿色，可以继续原任务。", "All bound identities are valid (green); the original task can continue.") : tr("完成动态验证码或 SSO 后，回到这里保存会话；不创建新任务。", "After completing the one-time code or SSO, come back here to save the session; no new task is created.") }}</span>
      <button class="button secondary" :disabled="!ready || Boolean(busy)" @click="emit('continue')"><RefreshCw :size="14" />{{ tr("会话有效，继续当前任务", "Session valid, continue this task") }}</button>
    </footer>
  </section>
</template>
