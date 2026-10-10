-- The DDL-time probe evaluation cannot catch this shape: `data->'k'` is SQL NULL
-- for any representative row whose object lacks key `k`, and the probe cannot know
-- which key the predicate names — so every downstream gate NULL-propagates and
-- never fires. The static operand check (`Expr::infer_type` + the shared legality
-- rules) types `data->'k'` as `jsonb` regardless of any value, and `jsonb ~~ text`
-- resolves to no operator. Postgres rejects it here, `42883`; accepting it left the
-- table unwritable (every INSERT failed in index maintenance) even across reopen.
-- @setup
CREATE TABLE jp2 (id INT, data JSONB);
-- @test
-- @expect-error: no jsonb ~~ text operator
CREATE INDEX jp2_i ON jp2 (id) WHERE data->'k' LIKE 'x%';
