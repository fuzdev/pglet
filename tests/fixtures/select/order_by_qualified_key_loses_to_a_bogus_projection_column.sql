-- **Error order, first of three.** Moving the sort keys below the projection changes which
-- error a doubly-wrong statement reports, and `transformSelectStmt`'s order is the
-- reference: FROM, target list, `WHERE`, `HAVING`, **sort**, group, distinct. So the
-- projection's missing column outranks the key's missing relation — 18.6 says `42703
-- column "bogusproj" does not exist`, where resolving the keys first reported the key's own
-- `42P01`.
--
-- The **qualified** key is what makes the pair measurable: a bare one was not judged at all
-- before this, so there was no second error to order against.
--
-- Its control is `select/order_by_qualified_key_beats_a_bogus_group_by_key.sql`, the one
-- neighbour the key still outranks — without it, "move the keys later" and "move them last"
-- are indistinguishable.
-- @setup
CREATE TABLE sk_ep (a INT, s TEXT);
INSERT INTO sk_ep VALUES (1, 'x'), (2, 'y');

-- @expect-error: column "bogusproj" does not exist (42703)
-- @test
SELECT bogusproj FROM sk_ep ORDER BY bogusrel.a;
