-- D120 (spec 05 sections 3.2 and 4.3): a knob nobody set follows its template default, an expression default worked
-- out at each evaluation, so a draft's `knobs` and an evaluation's `knobs` now hold only the values a person or the
-- agent set, and an evaluation records every knob's value as the engine took it, and where each came from.

ALTER TABLE template_evaluations
    -- { key: value } for every knob once the evaluation has ended; NULL until then, or when it never read the knobs.
    ADD COLUMN knob_values  JSONB,
    -- { key: "set" | "default" | "derived" } beside it.
    ADD COLUMN knob_sources JSONB;
