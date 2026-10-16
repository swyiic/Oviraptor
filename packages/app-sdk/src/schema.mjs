import { fail, jsonBytes, keys, plainObject } from './protocol.mjs';

const TYPES = ['object', 'array', 'string', 'integer', 'number', 'boolean', 'null'];
const COMMON = ['type', 'description', 'enum'];
const BY_TYPE = { object: ['properties', 'required', 'additionalProperties'], array: ['items', 'minItems', 'maxItems'],
  string: ['minLength', 'maxLength'], integer: ['minimum', 'maximum'], number: ['minimum', 'maximum'], boolean: [], null: [] };
const dangerous = key => ['__proto__', 'prototype', 'constructor'].includes(key);
const invalid = message => fail('app_schema_invalid', message);
const TOOL_LIMITS = Object.freeze({ properties: 64, enumValues: 32, arrayItems: 128, stringLength: 16384, bytes: 64 * 1024 });
const ACTION_LIMITS = Object.freeze({ properties: 256, enumValues: 256, arrayItems: 16384, stringLength: 1024 * 1024, bytes: 1024 * 1024 });
function schemaType(schema, limits) {
  const value = schema.type;
  if (typeof value === 'string' && TYPES.includes(value)) return { type: value, nullable: false };
  if (limits === ACTION_LIMITS && Array.isArray(value) && value.length === 2 && value.includes('null') && value.filter(item => item !== 'null' && TYPES.includes(item)).length === 1) return { type: value.find(item => item !== 'null'), nullable: true };
  invalid('Unsupported schema type or nullable union');
}
function integerBound(value, maximum, where) {
  if (!Number.isSafeInteger(value) || value < 0 || value > maximum) invalid(`${where} must be a bounded nonnegative integer`);
}
export function validateInputSchema(schema, depth = 0, limits = TOOL_LIMITS) {
  if (depth > 8 || !plainObject(schema)) invalid('Unsupported or deeply nested input schema');
  const { type } = schemaType(schema, limits);
  keys(schema, [...COMMON, ...BY_TYPE[type]], ['type'], 'inputSchema', 'app_schema_invalid');
  if (schema.description !== undefined && (typeof schema.description !== 'string' || schema.description.length > 1024)) invalid('Schema description exceeds its bound');
  if (schema.enum !== undefined) {
    if (type === 'object' || type === 'array') invalid('enum is supported only for scalar types');
    if (!Array.isArray(schema.enum) || schema.enum.length < 1 || schema.enum.length > limits.enumValues
      || schema.enum.some(v => !['string', 'number', 'boolean'].includes(typeof v) && v !== null)) invalid('enum requires bounded scalar values');
    for (const v of schema.enum) validateValue({ type: schema.type }, v, 'enum', true, limits);
    jsonBytes(schema.enum, 4096, 'app_schema_invalid');
    if (new Set(schema.enum.map(v => JSON.stringify(v))).size !== schema.enum.length) invalid('enum contains duplicates');
  }
  if (type === 'object') {
    if (!plainObject(schema.properties) || Object.keys(schema.properties).length > limits.properties || schema.additionalProperties !== false) invalid('Object schemas require closed bounded properties');
    for (const [key, child] of Object.entries(schema.properties)) {
      if (dangerous(key) || !key || key.length > 128) invalid('Invalid input property name');
      validateInputSchema(child, depth + 1, limits);
    }
    if (schema.required !== undefined && (!Array.isArray(schema.required) || schema.required.length > limits.properties
      || new Set(schema.required).size !== schema.required.length
      || schema.required.some(key => typeof key !== 'string' || !Object.hasOwn(schema.properties, key)))) invalid('required must name unique declared properties');
  }
  if (type === 'array') {
    if (!Object.hasOwn(schema, 'items') || !Object.hasOwn(schema, 'maxItems')) invalid('Arrays require items and maxItems');
    validateInputSchema(schema.items, depth + 1, limits);
    integerBound(schema.maxItems, limits.arrayItems, 'maxItems');
    if (schema.minItems !== undefined) integerBound(schema.minItems, schema.maxItems, 'minItems');
  }
  if (type === 'string') {
    if (schema.maxLength !== undefined) integerBound(schema.maxLength, limits.stringLength, 'maxLength');
    if (schema.minLength !== undefined) integerBound(schema.minLength, schema.maxLength ?? limits.stringLength, 'minLength');
  }
  if (['integer', 'number'].includes(type)) {
    for (const field of ['minimum', 'maximum']) if (schema[field] !== undefined
      && (typeof schema[field] !== 'number' || !Number.isFinite(schema[field])
        || (type === 'integer' && !Number.isSafeInteger(schema[field])))) invalid(`Invalid ${field}`);
    if (schema.minimum !== undefined && schema.maximum !== undefined && schema.minimum > schema.maximum) invalid('minimum exceeds maximum');
  }
  return schema;
}
function validateValue(schema, value, where, enumOnly = false, limits = TOOL_LIMITS) {
  const { type, nullable } = schemaType(schema, limits);
  if (nullable && value === null) {
    if (schema.enum && !schema.enum.some(item => item === null)) fail('app_arguments_invalid', `${where} is outside enum`);
    return;
  }
  const valid = type === 'object' ? plainObject(value) : type === 'array' ? Array.isArray(value)
    : type === 'null' ? value === null : type === 'integer' ? Number.isSafeInteger(value)
    : type === 'number' ? typeof value === 'number' && Number.isFinite(value) : typeof value === type;
  if (!valid) fail('app_arguments_invalid', `${where} has the wrong type`);
  if (enumOnly) return;
  if (schema.enum && !schema.enum.some(item => Object.is(item, value))) fail('app_arguments_invalid', `${where} is outside enum`);
  if (type === 'object') {
    for (const key of Object.keys(value)) if (!Object.hasOwn(schema.properties, key)) fail('app_arguments_invalid', `${where} has an undeclared field`);
    for (const key of schema.required ?? []) if (!Object.hasOwn(value, key)) fail('app_arguments_invalid', `${where} lacks a required field`);
    for (const [key, item] of Object.entries(value)) validateValue(schema.properties[key], item, `${where}.${key}`, false, limits);
  } else if (type === 'array') {
    if (value.length < (schema.minItems ?? 0) || value.length > schema.maxItems) fail('app_arguments_invalid', `${where} exceeds its item bounds`);
    value.forEach((item, i) => validateValue(schema.items, item, `${where}[${i}]`, false, limits));
  } else if (type === 'string') {
    if ([...value].length < (schema.minLength ?? 0) || [...value].length > (schema.maxLength ?? limits.stringLength)) fail('app_arguments_invalid', `${where} exceeds its text bounds`);
  } else if (type === 'number' || type === 'integer') {
    if (value < (schema.minimum ?? -Infinity) || value > (schema.maximum ?? Infinity)) fail('app_arguments_invalid', `${where} exceeds its numeric bounds`);
  }
}
export function validateToolArguments(schema, value) {
  validateInputSchema(schema);
  jsonBytes(value, 64 * 1024, 'app_arguments_invalid');
  validateValue(schema, value, 'arguments');
  return value;
}
export function validateActionSchema(schema) { return validateInputSchema(schema, 0, ACTION_LIMITS); }
export function validateActionArguments(schema, value) {
  validateActionSchema(schema);
  jsonBytes(value, ACTION_LIMITS.bytes, 'app_arguments_invalid');
  validateValue(schema, value, 'arguments', false, ACTION_LIMITS);
  return value;
}
