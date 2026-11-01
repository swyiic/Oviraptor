import { reactive, ref } from "vue";
import type { SentinelFuseEntry, SentinelScan } from "../types";
import { useI18n } from "../../../i18n";

export function useFuseDisposition(options: {
  save: (input: { id: number; verdict: string; note: string; evidence: string; archived: boolean }) => Promise<void>;
  remove: (id: number) => Promise<SentinelScan>;
  reload: () => Promise<void>;
  notify: (kind: "success" | "error", message: string) => void;
}) {
  const { tr } = useI18n();
  const fuseEditor = ref<SentinelFuseEntry>();
  const pendingFuseRemoval = ref<SentinelFuseEntry>();
  const fuseBusy = ref(false);
  const fuseForm = reactive({ verdict: "pending", note: "", evidence: "", archived: false });

  function editFuse(item: SentinelFuseEntry) {
    fuseEditor.value = item;
    Object.assign(fuseForm, {
      verdict: item.verdict || "pending",
      note: item.note || "",
      evidence: item.evidence || "",
      archived: item.archived,
    });
  }
  async function saveFuse(archive?: boolean) {
    if (!fuseEditor.value) return;
    fuseBusy.value = true;
    try {
      if (typeof archive === "boolean") fuseForm.archived = archive;
      await options.save({ id: fuseEditor.value.id, ...fuseForm });
      fuseEditor.value = undefined;
      await options.reload();
      options.notify("success", fuseForm.archived ? tr("停止记录已完成处置并归档", "Stop record resolved and archived") : tr("URL 处置记录已保存", "URL disposition saved"));
    } catch (error) {
      options.notify("error", String(error));
    } finally {
      fuseBusy.value = false;
    }
  }
  async function removeFuse() {
    if (!pendingFuseRemoval.value) return;
    fuseBusy.value = true;
    try {
      const retry = await options.remove(pendingFuseRemoval.value.id);
      pendingFuseRemoval.value = undefined;
      await options.reload();
      options.notify("success", tr(`URL 已移出已停止目标并进入自动重试任务 ${retry.id}`, `URL removed from stopped targets and queued for automatic retry in task ${retry.id}`));
    } catch (error) {
      options.notify("error", String(error));
    } finally {
      fuseBusy.value = false;
    }
  }
  return { fuseEditor, pendingFuseRemoval, fuseBusy, fuseForm, editFuse, saveFuse, removeFuse };
}
