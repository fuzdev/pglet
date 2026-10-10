-- The `ALTER TABLE … ADD CONSTRAINT … CHECK` route's own position for the render
-- probe. This route carries exactly **one** CHECK, so it has no source order to vary
-- and no reverse arrangement at all: the only thing measurable is whether the
-- pglet-only refusal loses to the gates 18.6 really raises. It does now — the
-- duplicate constraint name is `42710` on both engines, where the probe inside the
-- cook answered `0A000`.
-- @expect-error: the duplicate constraint name, not the interval CHECK (42710)
-- @setup
CREATE TABLE iv_al_dup (t TIMESTAMPTZ, CONSTRAINT iv_al_dup_d CHECK (t IS NOT NULL));
-- @test
ALTER TABLE iv_al_dup ADD CONSTRAINT iv_al_dup_d CHECK (t > now() - interval '1 day');
