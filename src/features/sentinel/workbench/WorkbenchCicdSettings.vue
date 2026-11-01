<script setup lang="ts">
import { GitBranch, ShieldCheck } from "@lucide/vue";
import { useI18n } from "../../../i18n";
import type { WorkbenchScanInput } from "../types";

defineProps<{
  form: Pick<WorkbenchScanInput, "ciProvider" | "repositoryUrl" | "branch" | "commitSha" | "buildId" | "environment" | "maxCritical" | "maxHigh" | "blockRelease">;
}>();

const { tr } = useI18n();
</script>

<template>
  <section class="workbench-mode-settings span-two">
    <header>
      <GitBranch :size="15" />
      <div><strong>{{ tr('流水线与发布门禁', 'Pipeline and release gate') }}</strong><small>{{ tr('CI/CD 只记录触发上下文并执行门禁，漏洞仍来自 SAST、SCA 和原生 Agent。', 'CI/CD only records the trigger context and enforces the gate; vulnerabilities still come from SAST, SCA and the native Agent.') }}</small></div>
    </header>
    <div class="workbench-fields nested-fields">
      <label class="field"><span>CI Provider</span>
        <select v-model="form.ciProvider">
          <option value="github">GitHub Actions</option>
          <option value="gitlab">GitLab CI</option>
          <option value="jenkins">Jenkins</option>
          <option value="azure">Azure Pipelines</option>
          <option value="other">Other</option>
        </select>
      </label>
      <label class="field"><span>{{ tr('仓库地址', 'Repository URL') }}</span><input v-model="form.repositoryUrl" placeholder="https://github.com/org/repo" /></label>
      <label class="field"><span>{{ tr('分支', 'Branch') }}</span><input v-model="form.branch" placeholder="main" /></label>
      <label class="field"><span>Commit SHA</span><input v-model="form.commitSha" placeholder="a89d20..." /></label>
      <label class="field"><span>Build / Pipeline ID</span><input v-model="form.buildId" placeholder="build-1024" /></label>
      <label class="field"><span>{{ tr('环境', 'Environment') }}</span><input v-model="form.environment" placeholder="production" /></label>
      <label class="field"><span>{{ tr('允许 Critical 数', 'Max critical allowed') }}</span><input v-model.number="form.maxCritical" type="number" min="0" /></label>
      <label class="field"><span>{{ tr('允许 High 数', 'Max high allowed') }}</span><input v-model.number="form.maxHigh" type="number" min="0" /></label>
      <label class="check-inline span-two"><input v-model="form.blockRelease" type="checkbox" /><ShieldCheck :size="14" />{{ tr('超出阈值时阻断发布', 'Block release when thresholds are exceeded') }}</label>
    </div>
  </section>
</template>
