-- **An error-order control.** `transformUpdateTargetList` transforms every right-hand
-- side first, then walks the targets in written order resolving each name
-- (`attnameAttNum`) and immediately coercing that assignment — so the two verdicts
-- interleave *per target* rather than running as two whole-list passes. Here the first
-- target's `42804` beats the second target's missing name; the mirrored spelling
-- (`SET nosuch = 1, b = 1`) is `42703` on both engines, which is the pair that
-- discriminates.
--
-- pglet checked the target names in the executor, as one pass ahead of the row loop,
-- which cannot report a later name after an earlier assignment's type error — so this
-- answered `42703`. The name check now sits beside the type verdict in the lowering.
-- @setup
CREATE TABLE acuo (id INT, b BOOL);

-- @expect-error: column "b" is of type boolean but expression is of type integer (42804)
-- @test
UPDATE acuo SET b = 1, nosuch = 1;
