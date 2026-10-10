-- The `INSERT … VALUES` list gates each value expression, but gating it *unresolved*
-- typed a bare literal as `text` and rejected `1 = '1'` as `integer = text` — a query
-- Postgres answers. Resolution now runs with the gate, so the unknown literal adopts its
-- sibling's type first.
-- @setup
CREATE TABLE ivl (id INT, b BOOL);
-- @test
INSERT INTO ivl VALUES (1, (1 = '1')), (2, ('2' = 3)) RETURNING id, b;
