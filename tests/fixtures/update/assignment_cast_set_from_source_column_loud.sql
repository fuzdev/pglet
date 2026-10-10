-- The `UPDATE … FROM` spelling: the right-hand side names a **source** relation's
-- column, so the gate's namespace has to be the statement's whole one — the target under
-- its effective name plus every relation the `FROM` introduces — and not the catalog.
-- Read it against `update/assignment_cast_set_yields_to_a_missing_source.sql`, which
-- pins the other half of the same namespace: a source the catalog cannot enumerate turns
-- the gate off rather than letting it answer from an invented type.
--
-- The source is **empty**, so the join matches nothing and no value is ever stored: an
-- eval-time refusal is not raised when there is no row, and this reported `UPDATE 0`
-- where 18.6 refuses the statement.
-- @setup
CREATE TABLE acuf (id INT, b BOOL);
CREATE TABLE acufs (n INT);
INSERT INTO acuf VALUES (1, FALSE);

-- @expect-error: column "b" is of type boolean but expression is of type integer (42804)
-- @test
UPDATE acuf SET b = s.n FROM acufs s;
