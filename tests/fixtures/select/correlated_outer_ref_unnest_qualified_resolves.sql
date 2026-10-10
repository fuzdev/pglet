-- The **qualified** half of the same over-rejection, and it was the coarser one: the seal
-- stopped the level walk entirely, so it also blocked references the un-enumerable
-- relation could not possibly shadow. `qra.y` names a relation the `unnest` item is not,
-- yet the walk halted and the reference stayed untyped — `42883 uuid = text` at eval time
-- where 18.6 returns the row.
--
-- Registering the `unnest` item under its alias with its exact columns removes the
-- question rather than carving an exception into it: the level is certain, so nothing has
-- to stop there. The bare spelling is
-- `select/correlated_outer_ref_unnest_level_resolves.sql`.
-- @setup
CREATE TABLE qra (id INT, y UUID);
INSERT INTO qra VALUES (1, '11111111-1111-1111-1111-111111111111');

-- @test
SELECT id FROM qra WHERE EXISTS (SELECT 1 FROM unnest(ARRAY['zz']) AS t(q) WHERE qra.y = '11111111-1111-1111-1111-111111111111');
