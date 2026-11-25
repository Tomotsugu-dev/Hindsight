import { expect, it } from "vitest";
import { version } from "../../../package.json";
import { getVersion } from "./api-app";

// 关于页的版本号跟着发版走，不写死
it("reports the version in package.json", async () => {
  expect(await getVersion()).toBe(version);
});
