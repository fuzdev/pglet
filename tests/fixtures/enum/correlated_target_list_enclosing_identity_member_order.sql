-- A sub-plan's target list naming an enclosing level's **enum** column orders by that
-- enum's member positions (`sad` < `ok` < `happy`) wherever the sub-plan's value is
-- compared — over an inner table carrying a same-named `text` column, over a one-column
-- one lacking the name, under `BETWEEN` and a fold. Read through the sub-plan's own
-- input it would be the label bytes (`EDGES.md` row 159).
-- @setup
CREATE TYPE eid_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE eid_te (m eid_mood, k INT);
CREATE TABLE eid_tt (m TEXT, k TEXT);
CREATE TABLE eid_tz (z TEXT);
INSERT INTO eid_te VALUES ('happy', 1), ('sad', 2);
INSERT INTO eid_tt VALUES ('x', 'y');
INSERT INTO eid_tz VALUES ('a');
-- @test
SELECT k,
       (SELECT o.m FROM eid_tt b) > 'ok' AS local_text_same_name,
       (SELECT o.m FROM eid_tz b) < 'ok' AS one_column_inner,
       (SELECT o.m FROM eid_tz b) BETWEEN 'sad' AND 'ok' AS between_labels,
       greatest((SELECT o.m FROM eid_tz b), 'ok')::text AS folded,
       coalesce((SELECT o.m FROM eid_tt b), 'ok') > 'ok' AS fold_compared,
       (SELECT o.m) > 'ok' AS no_from
  FROM eid_te o ORDER BY k;
