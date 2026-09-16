import { readFileSync } from "node:fs";
import assert from "node:assert/strict";
import { test } from "node:test";
import Ajv2020 from "ajv/dist/2020.js";
import type {
  ErrorResponse,
  Placement,
  EntryPage,
} from "../src/generated/api.ts";

const fixtures = JSON.parse(
  readFileSync(
    new URL(
      "../../crates/sct-protocol/tests/fixtures/wire.json",
      import.meta.url,
    ),
    "utf8",
  ),
) as {
  valid: Array<{ name: string; schema: string; data: unknown }>;
  invalid: Array<{ name: string; schema: string; data: unknown }>;
};
const schema = JSON.parse(
  readFileSync(new URL("../../contracts/schema.json", import.meta.url), "utf8"),
);
const ajv = new Ajv2020({ strict: true });
ajv.addFormat("uint64", { type: "number", validate: Number.isSafeInteger });
ajv.addFormat("uint32", {
  type: "number",
  validate: (value: number) =>
    Number.isInteger(value) && value >= 0 && value <= 4294967295,
});
ajv.addSchema(schema);
test("every generated schema compiles", () => {
  for (const name of Object.keys(schema.$defs)) {
    assert.ok(ajv.getSchema(`${schema.$id}#/$defs/${name}`), name);
  }
});
for (const [category, cases] of Object.entries(fixtures)) {
  for (const fixture of cases) {
    test(`${category}: ${fixture.name}`, () => {
      const validate = ajv.getSchema(`${schema.$id}#/$defs/${fixture.schema}`);
      assert.ok(validate, `missing schema ${fixture.schema}`);
      assert.equal(
        validate(fixture.data),
        category === "valid",
        JSON.stringify(validate.errors),
      );
      if (category === "valid")
        assert.deepEqual(
          JSON.parse(JSON.stringify(fixture.data)),
          fixture.data,
        );
    });
  }
}

// Typed consumption of the exact same validated fixture bytes as Rust.
function fixture<T>(name: string): T {
  const item = fixtures.valid.find((f) => f.schema === name)!;
  const validate = ajv.getSchema<T>(`${schema.$id}#/$defs/${name}`)!;
  assert.ok(validate(item.data));
  return item.data as T;
}
test("generated unions preserve retry, placement and cursor semantics", () => {
  const error = fixture<ErrorResponse>("ErrorResponse");
  assert.equal(error.retry.kind, "after");
  if (error.retry.kind === "after") assert.equal(error.retry.delay_ms, 1000);
  const placement = fixture<Placement>("Placement");
  assert.equal(placement.kind, "mirror");
  if (placement.kind === "mirror")
    assert.equal(placement.backend_ids.length, 2);
  const page = fixture<EntryPage>("EntryPage");
  assert.equal(page.next_cursor?.directory_revision, page.directory_revision);
});
