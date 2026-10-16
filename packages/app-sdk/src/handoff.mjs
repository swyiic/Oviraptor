const fail = () => { throw Object.assign(Error('app_page_handoff_invalid'), { code: 'app_page_handoff_invalid' }); };
/** Untrusted data for explicit review by a receiving application. No authority. */
export function readHandoff(value, projectId) {
  if (!value || typeof value !== 'object' || Array.isArray(value) ||
      Object.keys(value).sort().join(',') !== 'payload,projectId,protocol,purpose,sourceAppId' ||
      value.protocol !== 'oviraptor.handoff.v1' || value.purpose !== 'draft-only' || value.projectId !== projectId ||
      typeof value.sourceAppId !== 'string' || !value.sourceAppId || value.sourceAppId.length > 128 ||
      /[\u0000-\u0020\u007f]/u.test(value.sourceAppId) || !value.payload || typeof value.payload !== 'object' || Array.isArray(value.payload)) fail();
  let nodes = 0; const seen = new WeakSet(), encoder = new TextEncoder();
  function check(item, depth = 0) {
    if (++nodes > 8192 || depth > 8) fail();
    if (item === null || typeof item === 'boolean') return;
    if (typeof item === 'number') { if (!Number.isFinite(item)) fail(); return; }
    if (typeof item === 'string') { if (encoder.encode(item).length > 4096 || /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/u.test(item)) fail(); return; }
    if (typeof item !== 'object' || seen.has(item)) fail();
    seen.add(item);
    const array=Array.isArray(item),descriptors=Object.getOwnPropertyDescriptors(item),keys=Reflect.ownKeys(descriptors);
    if(!array&&![Object.prototype,null].includes(Object.getPrototypeOf(item)))fail();
    if(array?item.length>1024||keys.length!==item.length+1:keys.length>64)fail();
    for(const key of keys){
      if(array&&key==='length')continue;
      const descriptor=descriptors[key];
      if(typeof key!=='string'||!Object.hasOwn(descriptor,'value')||!descriptor.enumerable||['__proto__','prototype','constructor'].includes(key)||encoder.encode(key).length>128||/[\u0000-\u001f\u007f]/u.test(key)||array&&(!/^(0|[1-9]\d*)$/u.test(key)||Number(key)>=item.length))fail();
      check(descriptor.value,depth+1);
    }
    seen.delete(item);
  }
  check(value.payload);
  const raw = JSON.stringify(value); if (encoder.encode(JSON.stringify(value.payload)).length > 512 * 1024) fail();
  function freeze(item) { if (item && typeof item === 'object') { for (const child of Object.values(item)) freeze(child); Object.freeze(item); } return item; }
  return freeze(JSON.parse(raw));
}
