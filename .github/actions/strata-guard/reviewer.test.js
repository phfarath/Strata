const test = require('node:test');
const assert = require('node:assert/strict');

const reviewer = require('./reviewer.js');

const baseBlast = {
  modified_targets: ['crates/strata-memory/src/ipc.rs'],
  total_impacted_nodes: 1,
  highest_risk_score: 0.1,
  breaking_risks_count: 0,
  triggered_anti_patterns: [],
  safe_to_apply: true,
  blast_reports: [],
};

const hook = { known_failures: [], memories: [], query: 'test' };

test('completed low-risk analysis passes', () => {
  const report = reviewer.formatReport(
    { ...baseBlast, analysis_status: 'completed' },
    hook,
    'test'
  );

  assert.equal(report.analysisStatus, 'completed');
  assert.equal(report.safeToApply, true);
  assert.match(report.markdown, /Verdict: PASS/);
});

test('failed blast-radius never reports PASS and forces safeToApply false', () => {
  const report = reviewer.formatReport(
    {
      ...baseBlast,
      analysis_status: 'failed',
      analysis_error: 'blast-radius command failed',
      safe_to_apply: true,
    },
    hook,
    'test'
  );

  assert.equal(report.analysisStatus, 'failed');
  assert.equal(report.safeToApply, false);
  assert.doesNotMatch(report.markdown, /Verdict: PASS/);
  assert.match(report.markdown, /ANALYSIS FAILED/);
  assert.match(report.markdown, /blast-radius command failed/);
});

test('unavailable analysis never reports PASS and forces safeToApply false', () => {
  const report = reviewer.formatReport(
    {
      ...baseBlast,
      analysis_status: 'unavailable',
      analysis_error: 'blast-radius.json was not produced',
      safe_to_apply: true,
    },
    hook,
    'test'
  );

  assert.equal(report.analysisStatus, 'unavailable');
  assert.equal(report.safeToApply, false);
  assert.doesNotMatch(report.markdown, /Verdict: PASS/);
  assert.match(report.markdown, /ANALYSIS (FAILED|UNAVAILABLE)/);
});

test('partial analysis is explicit and preserves blast-radius safety', () => {
  const report = reviewer.formatReport(
    {
      ...baseBlast,
      analysis_status: 'partial',
      analysis_error: 'hook-prompt command failed',
    },
    hook,
    'test'
  );

  assert.equal(report.analysisStatus, 'partial');
  assert.equal(report.safeToApply, true);
  assert.match(report.markdown, /PARTIAL/);
  assert.match(report.markdown, /hook-prompt command failed/);
});

test('missing analysis_status is treated as completed for legacy output', () => {
  const report = reviewer.formatReport({ ...baseBlast }, hook, 'test');

  assert.equal(report.analysisStatus, 'completed');
  assert.equal(report.safeToApply, true);
});
