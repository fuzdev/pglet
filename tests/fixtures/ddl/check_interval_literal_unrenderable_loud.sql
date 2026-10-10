-- An `INTERVAL '…'` literal inside a CHECK is **accepted** by 18.6 and refused here
-- (`0A000`), because the parser carries an interval literal as an `Expr::Bound` and
-- the deparser has no source spelling for one. A pglet-only over-rejection, and the
-- alternative is worse than the refusal: `pglet::dump` renders every persisted
-- expression through one `Result`, so this CHECK used to make the **whole database**
-- undumpable — `pglet_server dump` exiting 1, with every other table's schema and
-- rows lost from the export. Measured on the tree before the render probe existed.
-- @expect-error: pglet-only — an interval literal in a stored expression cannot render
-- @test
CREATE TABLE ck_iv (t TIMESTAMPTZ, CHECK (t > now() - interval '1 day'));
