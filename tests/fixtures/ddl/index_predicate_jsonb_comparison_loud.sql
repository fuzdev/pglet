-- The DDL-time **static** half of the comparison gate, on the shape a
-- representative-row probe structurally cannot catch: `data->'k'` is NULL for
-- every synthetic row (the probe cannot know which key to synthesize), so the
-- eval-time gate NULL-propagates and never fires. Static inference has no such
-- hole — `data->'k'` is `jsonb` whatever the value is, and `jsonb = integer`
-- resolves to no operator (`42883`).
-- @setup
CREATE TABLE idx_jsonb_cmp (id INT, data JSONB);
-- @test
-- @expect-error: no jsonb = integer operator, caught statically at CREATE INDEX
CREATE INDEX idx_jsonb_cmp_p ON idx_jsonb_cmp (id) WHERE data->'k' = 5;
