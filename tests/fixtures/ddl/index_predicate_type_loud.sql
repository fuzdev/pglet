-- A partial-index predicate is type-checked at CREATE INDEX, where Postgres puts
-- it (`42883`) — not silently accepted and then failed on every later write. pglet
-- persisted the un-checked predicate, so `CREATE INDEX … WHERE int_col LIKE '…'`
-- was accepted and made the table unwritable: index maintenance evaluates the
-- stored predicate on each insert and the operand gate rejected it there.
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
-- @setup
CREATE TABLE idx_pred_ty (id INT, t TEXT);
-- @test
-- @expect-error: no integer ~~ text operator
CREATE INDEX idx_pred_ty_p ON idx_pred_ty (t) WHERE id LIKE '1%';
