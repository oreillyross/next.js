/* __next_internal_action_entry_do_not_use__ a,b,c */ import { createActionProxy } from "private-next-rsc-action-proxy";
export async function a() {}
export async function b() {}
export async function c() {}
function d() {}
function Foo() {
    async function e() {}
}
import { ensureServerEntryExports } from "private-next-rsc-action-validate";
ensureServerEntryExports([
    a,
    b,
    c
]);
createActionProxy("6e7bc104e4d6e7fda190c4a51be969cfd0be6d6d", null, a);
createActionProxy("d1f7eb64271d7c601dfef7d4d7053de1c2ca4338", null, b);
createActionProxy("1ab723c80dcca470e0410b4b2a2fc2bf21f41476", null, c);
