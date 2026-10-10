-- An index *key* expression is type-checked the same way: `CREATE INDEX … ((name
-- = 5))` over a text column is `42883`.
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
-- @setup
CREATE TABLE idx_cmp_key (id INT, name TEXT);
-- @test
-- @expect-error: no text = integer operator
CREATE INDEX idx_cmp_key_k ON idx_cmp_key ((name = 5));
