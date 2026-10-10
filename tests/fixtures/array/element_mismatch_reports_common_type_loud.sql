-- An `ARRAY[…]` element set with no common type is Postgres's `42804`, and it stays
-- `42804` when one of the elements happens to be an array: `boolean` and `integer[]` are
-- different type categories, which is what Postgres reports. pglet tested "is any element
-- an array?" *before* folding the common type, so every such pair came back as a nesting
-- complaint (`0A000`) instead — the right rejection under the wrong rule, and the wrong
-- SQLSTATE.
-- @setup
CREATE TABLE aem (b BOOL, ia INT4[]);
INSERT INTO aem VALUES (true, ARRAY[1,2]);
-- @expect-error: ARRAY element types are in different categories (42804)
-- @test
SELECT ARRAY[b, ia] FROM aem;
