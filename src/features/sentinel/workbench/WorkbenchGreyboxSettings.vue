<script setup lang="ts">
import { KeyRound } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { WorkbenchScanInput } from "../types";

defineProps<{
  form: Pick<WorkbenchScanInput, "environment" | "authProfileName" | "authType" | "authHeaderName" | "authValue">;
}>();

const { tr } = useI18n();
</script>

<template>
  <section class="workbench-mode-settings span-two">
    <header>
      <KeyRound :size="15" />
      <div>
        <strong>{{ tr('灰盒环境与手动认证备用', 'Greybox environment and manual auth fallback') }}</strong>
        <small>{{ tr('优先使用浏览器会话；这里只为无法打开浏览器登录的 API Token 场景保留。', 'Browser sessions are preferred; this is only kept for API token cases where browser login is not possible.') }}</small>
      </div>
    </header>
    <div class="workbench-fields nested-fields">
      <label class="field"><span>{{ tr('环境', 'Environment') }}</span><input v-model="form.environment" placeholder="staging" /></label>
      <label class="field"><span>{{ tr('认证配置名称', 'Auth profile name') }}</span><input v-model="form.authProfileName" :placeholder="tr('测试管理员会话', 'Test admin session')" /></label>
      <label class="field"><span>{{ tr('认证方式', 'Auth method') }}</span>
        <select v-model="form.authType">
          <option value="none">{{ tr('匿名', 'Anonymous') }}</option>
          <option value="cookie">Cookie</option>
          <option value="bearer">Bearer Token</option>
          <option value="header">{{ tr('自定义 Header', 'Custom header') }}</option>
        </select>
      </label>
      <label v-if="form.authType === 'header'" class="field"><span>{{ tr('Header 名称', 'Header name') }}</span><input v-model="form.authHeaderName" placeholder="X-API-Key" /></label>
      <label v-if="form.authType !== 'none'" class="field span-two"><span>{{ tr('临时会话值', 'Temporary session value') }}</span>
        <input v-model="form.authValue" type="password" autocomplete="off"
          :placeholder="form.authType === 'cookie' ? 'session=...' : form.authType === 'bearer' ? 'eyJ...' : 'Header value'" />
      </label>
    </div>
  </section>
</template>
