-- The `pg_attrdef` exemption is scoped to the sequence's **own** column, and the column
-- half of that match is load-bearing on its own. A plain sibling may draw from an
-- identity column's sequence — both engines accept the `CREATE TABLE` — and its default
-- is then an ordinary NORMAL edge, so dropping the identity column is 2BP01 and names it.
-- An exemption matching on the owner *table* alone hides that sibling, and the drop then
-- succeeds and leaves `b` defaulting from a sequence the catalog no longer holds: every
-- write 42P01, and a dump emitting the `DEFAULT nextval(...)` with no `CREATE SEQUENCE`.
-- @setup
CREATE TABLE dcsd_t (id INT GENERATED ALWAYS AS IDENTITY, b INT DEFAULT nextval('dcsd_t_id_seq'));
-- @test
ALTER TABLE dcsd_t DROP COLUMN id;
-- @expect-error: the sibling's default depends on the identity's sequence (2BP01)
