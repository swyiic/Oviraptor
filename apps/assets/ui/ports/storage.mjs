// Opaque frames cannot access browser localStorage or sessionStorage. These
// presentation preferences currently live only for this page lifetime. Native
// app-owned preference persistence is a separate explicit operator contract.
function area(){const values=new Map();return Object.freeze({getItem:key=>values.get(String(key))??null,setItem:(key,value)=>values.set(String(key),String(value)),removeItem:key=>values.delete(String(key))});}
export const assetLocalStorage=area();
export const assetSessionStorage=area();
