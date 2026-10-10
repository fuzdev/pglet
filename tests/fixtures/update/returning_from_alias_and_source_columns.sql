-- One `RETURNING` list naming **both** relations of an `UPDATE … FROM`, with the target
-- aliased — `RETURNING o.v, s.k`.
--
-- The spelling the widened namespace exists for, and the one that separates "the source
-- was added" from "the target was replaced": either relation dropping out of the scope
-- reddens exactly one of the two items. The target is aliased, so `o` is the only name
-- that reaches it (`rao_t.v` is `42P01` — see
-- `update/returning_alias_hides_table_name.sql`), and the two relations carry different
-- values under differently-named columns, so a mis-bound reference shows in the data.
-- @setup
CREATE TABLE rao_t (id INT, v TEXT);
CREATE TABLE rao_s (id INT, sv TEXT, k INT);
INSERT INTO rao_t VALUES (1, 'TARGET');
INSERT INTO rao_s VALUES (1, 'SOURCE', 7);

-- @test
UPDATE rao_t o SET v = 'hit' FROM rao_s s WHERE s.id = o.id RETURNING o.v, s.sv, s.k;
