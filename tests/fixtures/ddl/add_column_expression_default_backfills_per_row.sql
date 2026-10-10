-- An `ADD COLUMN` backfill re-resolves a **volatile** default per existing row, and
-- an expression is volatile when it *contains* a volatile leaf — reading only the top
-- node puts `nextval('s') + 1` on the stable side and writes one value into every row.
-- @setup
CREATE SEQUENCE addcol_seq;
CREATE TABLE addcol_t (k INT);
INSERT INTO addcol_t VALUES (1), (2), (3);
ALTER TABLE addcol_t ADD COLUMN v INT DEFAULT nextval('addcol_seq') + 100;
-- @test
SELECT k, v FROM addcol_t ORDER BY k;
