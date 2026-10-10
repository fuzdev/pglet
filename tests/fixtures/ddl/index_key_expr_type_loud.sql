-- Same for an expression index's key part: `(id || id)` has no `||` operator over
-- two integers, so `CREATE INDEX` fails `42883` rather than persisting a key
-- expression that errors on every insert.
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
-- @setup
CREATE TABLE idx_key_ty (id INT);
-- @test
-- @expect-error: no integer || integer operator
CREATE INDEX idx_key_ty_i ON idx_key_ty ((id || id));
