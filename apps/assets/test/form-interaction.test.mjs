import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {createRequire} from 'node:module';
const require=createRequire(import.meta.url),ts=require('typescript'),exports={};
const source=readFileSync(new URL('../../../src/features/assets/formInteraction.ts',import.meta.url),'utf8');
new Function('exports',ts.transpileModule(source,{compilerOptions:{module:ts.ModuleKind.CommonJS,target:ts.ScriptTarget.ES2020}}).outputText)(exports);
const submit=exports.submitAssetsFormOnEnter;
function event(over={}){return {key:'Enter',target:{tagName:'INPUT',type:'text'},preventDefault(){this.prevented=true;this.defaultPrevented=true;},...over};}
test('normal input Enter submits once and prevents duplicate implicit form handling',()=>{let calls=0;const e=event();submit(e,()=>calls++);submit(e,()=>calls++);assert.equal(calls,1);assert.equal(e.prevented,true);});
test('textarea select button file checkbox composition and modified Enter retain their own keyboard behavior',()=>{
 let calls=0;for(const over of [{target:{tagName:'TEXTAREA'}},{target:{tagName:'SELECT'}},{target:{tagName:'BUTTON'}},{target:{tagName:'INPUT',type:'file'}},{target:{tagName:'INPUT',type:'checkbox'}},{isComposing:true},{repeat:true},{altKey:true},{ctrlKey:true},{metaKey:true},{shiftKey:true},{key:'Escape'}]){const e=event(over);submit(e,()=>calls++);assert.equal(e.prevented,undefined,JSON.stringify(over));}assert.equal(calls,0);
});
test('busy disabled and read-only controls do not bypass the rendered action gate',()=>{
 let calls=0;submit(event(),()=>calls++,true);submit(event({target:{tagName:'INPUT',type:'text',disabled:true}}),()=>calls++);submit(event({target:{tagName:'INPUT',type:'text',readOnly:true}}),()=>calls++);assert.equal(calls,0);
});
