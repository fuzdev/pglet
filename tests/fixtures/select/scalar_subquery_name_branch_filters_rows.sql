-- What the over-refusal cost was a row, not a label: the same branch set in a `WHERE`
-- predicate. A fixture that pins only the refusing direction cannot tell a gate that
-- refuses what 18.6 refuses from one that refuses more, which is how the over-refusal
-- shipped.
-- @setup
CREATE TABLE ssnw (i INT, tx TEXT);
INSERT INTO ssnw VALUES (1, 'aaa'), (2, 'public');
-- @test
SELECT i FROM ssnw WHERE COALESCE((SELECT current_schema), tx) = 'public' ORDER BY i;
