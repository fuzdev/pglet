-- A `"…"`-quoted relation name is **exact**, so it does not find a table created from
-- an unquoted mixed-case spelling: `CREATE TABLE Foo` registers `foo` and `"Foo"`
-- names nothing (`42P01` on both engines).
-- This is the discriminator against **both** plausible wrong implementations at once,
-- which is why it is worth a fixture of its own. Fold nothing (the old engine) and
-- `Foo` is registered as `Foo`, so `"Foo"` resolves; fold everything, quoted included,
-- and `"Foo"` becomes `foo`, so it resolves again. Only the split rule refuses.
-- @setup
CREATE TABLE Foo (a INT);

-- @test
SELECT * FROM "Foo";
-- @expect-error: a quoted name is exact and `Foo` was registered as `foo` (42P01)
