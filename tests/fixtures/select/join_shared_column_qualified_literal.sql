-- Two FROM relations sharing a column name, and a **qualified** reference to one of
-- them compared against a bare string literal. The coercion pass used to look a
-- column up by its bare name in a map merged across every FROM relation, where a
-- later table overwrote an earlier one — so `jsq_a.x = '1'` cast the literal to
-- `jsq_b`'s `text`, and the gate (which resolved `jsq_a.x` correctly as `integer`)
-- then rejected the pair as `integer = text`. A `42883` on a query Postgres runs.
-- @setup
CREATE TABLE jsq_a (x INT);
CREATE TABLE jsq_b (x TEXT, label TEXT);
INSERT INTO jsq_a VALUES (1), (2);
INSERT INTO jsq_b VALUES ('z', 'only');
-- @test
SELECT jsq_a.x, jsq_b.label FROM jsq_a JOIN jsq_b ON true WHERE jsq_a.x = '1';
