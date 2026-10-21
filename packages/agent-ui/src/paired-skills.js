/** Build a one-click include request for application paired_skills (reference-only). */
export function pairedSkillsIncludeBody({ names, conversationId, includeInSkillsRead = true } = {}) {
  const body = { includeInSkillsRead: Boolean(includeInSkillsRead) };
  if (Array.isArray(names)) body.names = names;
  if (includeInSkillsRead) {
    if (conversationId === null || conversationId === undefined || conversationId === '') {
      throw Object.assign(Error('Choose a conversation before including paired Skills in skills_read.'), { code: 'host_paired_skill_conversation_required' });
    }
    body.conversationId = conversationId;
  }
  return body;
}

export function pairedSkillsSummary(app) {
  const preview = app?.pairedSkillsPreview;
  if (preview?.skills) return preview.skills;
  if (Array.isArray(app?.pairedSkills)) return app.pairedSkills.map(item => ({
    name: item.name, source: item.source, required: Boolean(item.required),
    status: 'declared', importable: item.source !== 'bundled', inSkillsReadScope: false, skillId: null, excerpt: null,
  }));
  return [];
}
