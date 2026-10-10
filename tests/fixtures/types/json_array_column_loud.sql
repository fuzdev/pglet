-- `json[]` is a member of the refusal, not a consequence of it: the array spelling
-- resolves its element through the same table the scalar one does, so it answers `42704`
-- at the same position. 18.6 accepts the column (atttypid 199).
--
-- **Documentary, not discriminating** — 18.6 accepts, so any pglet error passes here;
-- `crates/pglet/tests/json_type_refused.rs` pins the SQLSTATE.
-- @setup

-- @test
-- @expect-error: type "json" is not supported
CREATE TABLE json_arr_col (a json[]);
