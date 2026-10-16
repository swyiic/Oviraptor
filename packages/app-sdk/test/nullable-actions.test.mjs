import test from 'node:test';
import assert from 'node:assert/strict';
import { validateActionArguments, validateActionSchema, validateToolArguments } from '../src/schema.mjs';

const object = properties => ({ type: 'object', properties, additionalProperties: false });
test('operator nullable types preserve explicit null and closed nested fields', () => {
  const schema = object({ note: { type: ['string', 'null'], minLength: 2, maxLength: 4 },
    selection: { ...object({ id: { type: ['integer', 'null'], minimum: 1 } }), type: ['object', 'null'] },
    ids: { type: ['array', 'null'], items: { type: ['integer', 'null'], minimum: 1 }, maxItems: 3 },
    choice: { type: ['string', 'null'], enum: ['yes', null] } });
  for (const value of [{ note: null, selection: null, ids: null, choice: null }, { note: 'yes', selection: { id: null }, ids: [1, null], choice: 'yes' }]) assert.deepEqual(validateActionArguments(schema, value), value);
  for (const value of [{ note: 'x' }, { selection: { extra: 1 } }, { selection: { id: 0 } }, { ids: [1, 2, 3, 4] }, { choice: 'no' }]) assert.throws(() => validateActionArguments(schema, value), { code: 'app_arguments_invalid' });
});
test('nullable operator schemas cannot introduce unrestricted unions or expand model tool contracts', () => {
  for (const type of [['string', 'number'], ['null', 'null'], ['string', 'null', 'integer'], ['unknown', 'null'], []]) assert.throws(() => validateActionSchema(object({ value: { type } })), { code: 'app_schema_invalid' });
  assert.throws(() => validateActionSchema(object({ value: { type: ['object', 'null'], properties: {}, additionalProperties: true } })), { code: 'app_schema_invalid' });
  assert.throws(() => validateToolArguments(object({ value: { type: ['integer', 'null'] } }), { value: null }), { code: 'app_schema_invalid' });
});

test('complete closed operator settings support the actual 76-field profile while model schemas keep 64 fields', () => {
  const properties = Object.fromEntries(Array.from({length:76},(_,i)=>[`field_${i}`,{type:'string'}]));
  const schema = {...object(properties),required:Object.keys(properties)},value=Object.fromEntries(Object.keys(properties).map(key=>[key,'saved']));
  assert.deepEqual(validateActionArguments(schema,value),value);
  assert.throws(()=>validateToolArguments(schema,value),{code:'app_schema_invalid'});
  const excessive=object(Object.fromEntries(Array.from({length:257},(_,i)=>[`field_${i}`,{type:'boolean'}])));
  assert.throws(()=>validateActionSchema(excessive),{code:'app_schema_invalid'});
});
