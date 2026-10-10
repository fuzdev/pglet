-- A qualified `RETURNING` reference to a `FROM` source that **does not exist**: `42P01`
-- on both engines, for different reasons that happen to share a SQLSTATE — 18.6 refuses
-- the relation (`relation "rms_nosuch" does not exist`), pglet refuses the qualifier
-- (`missing FROM-clause entry for table "s"`), because a `Scan` naming a table the
-- catalog lacks is registered in the `RETURNING` namespace as **nothing at all**.
--
-- That registration choice is the subject, and it is the one this fixture exists to pin.
-- The tempting simplification is to build the namespace from `Plan::output_columns_scoped`,
-- which every other projecting node already uses — but that registers a missing table as
-- an *empty* relation, and an empty relation makes this statement `42703` at plan time,
-- ahead of the `42P01` both engines report. Same statement, same build, one word of
-- difference in the walk, and the SQLSTATE moves; nothing else in the corpus separates
-- the two.
--
-- Its wildcard siblings (`returning_from_wildcard_yields_to_missing_source.sql` and its
-- qualified control) stay green under that wrong walk, because a `*` over an empty
-- relation expands to nothing and lets the execute-time `42P01` through unchanged.
-- @setup
CREATE TABLE rms_t (id INT, v TEXT);
INSERT INTO rms_t VALUES (1, 'TARGET');

-- @test
-- @expect-error: 42P01 — the FROM source does not exist, so nothing named `s` is in scope
UPDATE rms_t SET id = 1 FROM rms_nosuch s WHERE s.id = rms_t.id RETURNING s.id;
