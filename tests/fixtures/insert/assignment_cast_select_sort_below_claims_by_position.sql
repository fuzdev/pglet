-- **A false-rejection control — do not trim it**, and the pair-mate of
-- `assignment_cast_select_sort_below_claims_the_named_entry.sql`: the two statements
-- differ only in the target column order, and 18.6 accepts this one and refuses that
-- one. `ORDER BY p` claims the first entry alone, so the second stays `unknown` and
-- assigns into the `BOOL` column.
--
-- The two entries are the **same expression** under different aliases, which is why a
-- claim read back off the finished plan cannot serve: sort-below rewrites the key into
-- the entry it named, and `'t'` matches both. Marking every equal entry refuses this
-- statement; marking none writes the row its mirror shows 18.6 refusing. The position
-- is recorded at the lowering for exactly this pair.
-- @setup
CREATE TABLE acdp (t TEXT, b BOOL);
CREATE TABLE acdpsrc (n INT);
INSERT INTO acdpsrc VALUES (1), (2);
-- @test
INSERT INTO acdp (t, b) SELECT 't' AS p, 't' AS q FROM acdpsrc ORDER BY p, n RETURNING t, b;
