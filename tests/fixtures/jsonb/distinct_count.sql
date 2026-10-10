-- DISTINCT over a jsonb column routes through the ordering-backed equality, so the
-- duplicate `1` folds and the distinct set spans multiple type ranks (a JSON `null`
-- value is a real value here, not SQL NULL).
-- @setup
CREATE TABLE j (v JSONB);
INSERT INTO j VALUES ('1'), ('1'), ('2'), ('"a"'), ('null');
-- @test
SELECT count(DISTINCT v) AS n FROM j;
