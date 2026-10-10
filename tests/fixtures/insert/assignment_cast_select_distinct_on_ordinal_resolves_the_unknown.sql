-- The claiming half of `assignment_cast_select_distinct_on_a_source_column_does_not_resolve.sql`:
-- `transformDistinctOnClause` groups on the ON expressions, so an ON key that **names a
-- target-list entry** resolves that entry's `unknown` exactly as a `GROUP BY` ordinal
-- does. `42804` on 18.6, where the source-column spelling next door is accepted.
--
-- It is also the fixture that pins *where* the claim is read. A `DISTINCT ON` ordinal
-- does not survive lowering: `resolve_output_position` canonicalizes it into a by-name
-- reference to the entry, and over the unnamed `?column?` entries this statement has,
-- the name is not distinguishing. The claim is taken from the written ordinal before
-- that, beside the `ORDER BY` keys it is merged with.
-- @setup
CREATE TABLE acdn (id INT, b BOOL);
CREATE TABLE acdnsrc (n INT);
INSERT INTO acdnsrc VALUES (1), (2);

-- @expect-error: column "b" is of type boolean but expression is of type text (42804)
-- @test
INSERT INTO acdn (b) SELECT DISTINCT ON (1) 't' FROM acdnsrc ORDER BY 1, n;
