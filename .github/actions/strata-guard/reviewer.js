const fs = require('fs');
const path = require('path');

const ANALYSIS_COMPLETED = 'completed';
const ANALYSIS_PARTIAL = 'partial';
const ANALYSIS_FAILED = 'failed';
const ANALYSIS_UNAVAILABLE = 'unavailable';
const ANALYSIS_STATUSES = new Set([
  ANALYSIS_COMPLETED,
  ANALYSIS_PARTIAL,
  ANALYSIS_FAILED,
  ANALYSIS_UNAVAILABLE,
]);

function normalizeAnalysisStatus(value) {
  if (typeof value !== 'string') return ANALYSIS_COMPLETED;
  const normalized = value.toLowerCase();
  return ANALYSIS_STATUSES.has(normalized) ? normalized : ANALYSIS_PARTIAL;
}

/**
 * Parses and formats Strata analysis into a structured PR review markdown document.
 */
function formatReport(blastRadius, hookPrompt, prTitle) {
  const analysisStatus = normalizeAnalysisStatus(blastRadius.analysis_status);
  const analysisError = typeof blastRadius.analysis_error === 'string'
    ? blastRadius.analysis_error
    : '';
  const analysisFailed = analysisStatus === ANALYSIS_FAILED || analysisStatus === ANALYSIS_UNAVAILABLE;
  const analysisPartial = analysisStatus === ANALYSIS_PARTIAL;

  let riskScore = 0.0;
  if (typeof blastRadius.highest_risk_score === 'number') {
    riskScore = blastRadius.highest_risk_score;
  } else if (typeof blastRadius.overall_risk_score === 'number') {
    riskScore = blastRadius.overall_risk_score;
  }

  const riskPct = Math.round(riskScore * 100);

  let riskLevel = 'Low';
  let riskEmoji = '🟢';
  let riskBadge = '🟢 **LOW RISK**';

  if (riskScore >= 0.80) {
    riskLevel = 'Critical';
    riskEmoji = '🚨';
    riskBadge = '🚨 **CRITICAL RISK**';
  } else if (riskScore >= 0.50) {
    riskLevel = 'Elevated';
    riskEmoji = '⚠️';
    riskBadge = '⚠️ **ELEVATED RISK**';
  } else if (riskScore >= 0.25) {
    riskLevel = 'Moderate';
    riskEmoji = '🟡';
    riskBadge = '🟡 **MODERATE RISK**';
  }

  if (analysisFailed) {
    riskLevel = 'Unavailable';
    riskEmoji = '⚪';
    riskBadge = '⚪ **ANALYSIS UNAVAILABLE**';
  }

  // Aggregate invariants
  const invariants = new Set();
  if (Array.isArray(blastRadius.triggered_invariants)) {
    blastRadius.triggered_invariants.forEach(inv => invariants.add(inv));
  }
  if (Array.isArray(blastRadius.blast_reports)) {
    for (const r of blastRadius.blast_reports) {
      if (Array.isArray(r.triggered_invariants)) {
        r.triggered_invariants.forEach(inv => invariants.add(inv));
      }
    }
  }

  // Aggregate anti-patterns from blast radius
  const triggeredAntiPatterns = new Set(blastRadius.triggered_anti_patterns || []);
  if (Array.isArray(blastRadius.blast_reports)) {
    for (const r of blastRadius.blast_reports) {
      if (Array.isArray(r.triggered_anti_patterns)) {
        r.triggered_anti_patterns.forEach(ap => triggeredAntiPatterns.add(ap));
      }
    }
  }

  // Known failures from cognitive memory hook
  const knownFailures = (hookPrompt && Array.isArray(hookPrompt.known_failures))
    ? hookPrompt.known_failures
    : [];

  // Recommendations
  const recommendations = new Set();
  if (Array.isArray(blastRadius.recommendations)) {
    blastRadius.recommendations.forEach(rec => recommendations.add(rec));
  }
  if (Array.isArray(blastRadius.blast_reports)) {
    for (const r of blastRadius.blast_reports) {
      if (Array.isArray(r.recommendations)) {
        r.recommendations.forEach(rec => recommendations.add(rec));
      }
    }
  }

  // Aggregate impacts
  const allDirectImpacts = [];
  const allTransitiveImpacts = [];
  if (Array.isArray(blastRadius.direct_impacts)) {
    allDirectImpacts.push(...blastRadius.direct_impacts);
  }
  if (Array.isArray(blastRadius.transitive_impacts)) {
    allTransitiveImpacts.push(...blastRadius.transitive_impacts);
  }
  if (Array.isArray(blastRadius.blast_reports)) {
    for (const r of blastRadius.blast_reports) {
      if (Array.isArray(r.direct_impacts)) {
        allDirectImpacts.push(...r.direct_impacts);
      }
      if (Array.isArray(r.transitive_impacts)) {
        allTransitiveImpacts.push(...r.transitive_impacts);
      }
    }
  }

  const modifiedTargets = blastRadius.modified_targets || [];
  const totalImpacted = blastRadius.total_impacted_nodes !== undefined
    ? blastRadius.total_impacted_nodes
    : (allDirectImpacts.length + allTransitiveImpacts.length);
  const breakingRisks = blastRadius.breaking_risks_count !== undefined
    ? blastRadius.breaking_risks_count
    : (allDirectImpacts.filter(i => i.is_breaking_risk).length + allTransitiveImpacts.filter(i => i.is_breaking_risk).length);
  let safeToApply = blastRadius.safe_to_apply !== undefined
    ? Boolean(blastRadius.safe_to_apply)
    : (riskScore < 0.75 && breakingRisks === 0 && invariants.size === 0);
  if (analysisFailed) {
    safeToApply = false;
  }

  const lines = [];

  // Marker for sticky comment identification
  lines.push('<!-- strata-guard-report -->');
  lines.push('## 🛡️ Strata Guard — Cognitive Architecture & PR Review');
  lines.push('');
  lines.push('> Automated architectural risk assessment powered by **Strata Cognitive Memory & Causal World Model**.');
  lines.push('');

  lines.push('### 🧪 Analysis Status');
  lines.push('');
  if (analysisFailed) {
    lines.push('> [!CAUTION]');
    lines.push('> **Analysis failed or unavailable.** This report is not a PASS: `safe_to_apply` is forced to `false`.');
    if (analysisError) {
      lines.push(`> \n> **Reason**: ${analysisError}`);
    }
  } else if (analysisPartial) {
    lines.push('> [!WARNING]');
    lines.push('> **Partial analysis.** Blast-radius results are available, but some cognitive checks could not run.');
    if (analysisError) {
      lines.push(`> \n> **Reason**: ${analysisError}`);
    }
  } else {
    lines.push('✅ **Blast-radius analysis completed successfully.**');
  }
  lines.push('');

  // 1. 🎯 Causal Blast Radius & Risk Level
  lines.push('### 🎯 Causal Blast Radius & Risk Level');
  lines.push('');
  lines.push('| Metric | Value | Status |');
  lines.push('| :--- | :--- | :--- |');
  lines.push(`| **Pre-Code Risk Score** | \`${riskPct}%\` | ${riskBadge} |`);
  lines.push(`| **Risk Classification** | **${riskLevel}** | ${riskEmoji} |`);
  lines.push(`| **Modified Targets** | \`${modifiedTargets.length}\` files | 📁 |`);
  lines.push(`| **Impacted Architectural Nodes** | \`${totalImpacted}\` nodes | 🌐 |`);
  lines.push(`| **Breaking Change Risks** | \`${breakingRisks}\` detected | ${breakingRisks > 0 ? '⚠️ Warning' : '✅ Clean'} |`);
  lines.push('');

  if (modifiedTargets.length > 0) {
    lines.push('<details>');
    lines.push(`<summary><b>🔍 Touched Targets (${modifiedTargets.length})</b></summary>`);
    lines.push('');
    modifiedTargets.forEach(t => lines.push(`- \`${t}\``));
    lines.push('</details>');
    lines.push('');
  }

  if (allDirectImpacts.length > 0 || allTransitiveImpacts.length > 0) {
    lines.push('<details>');
    lines.push(`<summary><b>🌲 Causal Dependency & Impact Ripple Tree (${allDirectImpacts.length} Direct, ${allTransitiveImpacts.length} Transitive)</b></summary>`);
    lines.push('');
    lines.push('| Node / Component | Kind | Distance | Coupling Weight | Breaking Risk |');
    lines.push('| :--- | :--- | :--- | :--- | :--- |');

    const seenNodes = new Set();
    allDirectImpacts.forEach(node => {
      const id = node.node_id || node.name;
      if (seenNodes.has(id)) return;
      seenNodes.add(id);
      const breaking = node.is_breaking_risk ? '⚠️ **YES**' : 'No';
      const weight = Math.round((node.cumulative_weight || 0) * 100);
      lines.push(`| \`${node.name || node.node_id}\` | \`${node.kind || 'File'}\` | Direct (d=1) | \`${weight}%\` | ${breaking} |`);
    });

    allTransitiveImpacts.forEach(node => {
      const id = node.node_id || node.name;
      if (seenNodes.has(id)) return;
      seenNodes.add(id);
      const breaking = node.is_breaking_risk ? '⚠️ **YES**' : 'No';
      const weight = Math.round((node.cumulative_weight || 0) * 100);
      lines.push(`| \`${node.name || node.node_id}\` | \`${node.kind || 'Module'}\` | Transitive (d=${node.distance || 2}) | \`${weight}%\` | ${breaking} |`);
    });

    lines.push('');
    lines.push('</details>');
    lines.push('');
  }

  // 2. 🧠 Invariants & High-Importance Architectural Rules Checked
  lines.push('### 🧠 Invariants & High-Importance Architectural Rules Checked');
  lines.push('');
  if (invariants.size > 0) {
    lines.push('> [!CAUTION]');
    lines.push('> The following strict architectural contract invariants are triggered or impacted by this PR:');
    lines.push('>');
    Array.from(invariants).forEach(inv => {
      lines.push(`> - **${inv}**`);
    });
  } else {
    lines.push('✅ **No contract invariants breached.** All high-importance architectural contracts and constraints remain intact.');
  }
  lines.push('');

  // 3. ⚠️ Known Failure Anti-Patterns & Mitigations
  lines.push('### ⚠️ Known Failure Anti-Patterns & Mitigations');
  lines.push('');
  const hasAntiPatterns = triggeredAntiPatterns.size > 0 || knownFailures.length > 0;
  if (hasAntiPatterns) {
    if (triggeredAntiPatterns.size > 0) {
      lines.push('**Triggered Architectural Anti-Patterns:**');
      Array.from(triggeredAntiPatterns).forEach(ap => {
        lines.push(`- ⚠️ **${ap}**`);
      });
      lines.push('');
    }

    if (knownFailures.length > 0) {
      lines.push('**Relevant Known Failure Signatures & Prescribed Mitigations:**');
      lines.push('');
      knownFailures.forEach(f => {
        lines.push(`> [!WARNING]`);
        lines.push(`> #### ${f.pattern_name || f.signature}`);
        lines.push(`> **Signature**: \`${f.signature}\` &nbsp;|&nbsp; **Severity**: \`${f.severity || 'Medium'}\` &nbsp;|&nbsp; **Occurrences**: \`${f.occurrences || 1}\``);
        if (f.description) {
          lines.push(`> \n> **Description**: ${f.description}`);
        }
        if (f.mitigation) {
          lines.push(`> \n> 🛡️ **Mitigation**: *${f.mitigation}*`);
        }
        lines.push('');
      });
    }
  } else {
    lines.push('✅ **No known failure anti-patterns detected.** PR scope and touched modules do not match recurring failure signatures.');
  }
  lines.push('');

  // 4. 💡 Recommended Action & Verdict
  lines.push('### 💡 Recommended Action');
  lines.push('');
  const verdictPass = !analysisFailed && safeToApply && riskScore < 0.50 && breakingRisks === 0 && invariants.size === 0;

  if (analysisFailed) {
    lines.push('#### 🔴 **Verdict: ANALYSIS FAILED — NOT A PASS**');
    lines.push('Strata Guard could not complete the causal blast-radius analysis, so safety cannot be asserted. Do not treat this report as approval.');
    if (analysisError) {
      lines.push('');
      lines.push(`**Analysis error**: ${analysisError}`);
    }
  } else if (verdictPass && analysisPartial) {
    lines.push('#### 🟡 **Verdict: PASS (PARTIAL ANALYSIS)**');
    lines.push('Changes are within safe architectural thresholds, but some cognitive checks were unavailable. Review the analysis warning above.');
  } else if (verdictPass) {
    lines.push('#### 🟢 **Verdict: PASS**');
    lines.push('Changes are within safe architectural thresholds. Contract invariants are satisfied, no high-risk ripple coupling was detected, and all memory guardrails passed.');
  } else {
    lines.push('#### ⚠️ **Verdict: ATTENTION NEEDED**');
    lines.push('This pull request introduces elevated ripple risk, potential contract violations, or touches sensitive architectural boundaries. Please review recommendations prior to merging.');
  }
  lines.push('');

  if (recommendations.size > 0) {
    lines.push('**Actionable Recommendations:**');
    Array.from(recommendations).forEach(rec => {
      lines.push(`- 👉 ${rec}`);
    });
    lines.push('');
  } else {
    lines.push('- 👉 Verify with standard CI tests (`cargo test --workspace`).');
    lines.push('');
  }

  // Footer
  lines.push('---');
  lines.push('*Automated review by [Strata Guard](https://github.com/phfarath/strata) • Cognitive Memory & Causal World Model Engine*');

  return {
    markdown: lines.join('\n'),
    riskLevel,
    riskScore,
    riskPct,
    safeToApply,
    analysisStatus,
    analysisError
  };
}

/**
 * Main entrypoint called by actions/github-script.
 */
async function runGuard({ github, context, core }) {
  try {
    const reportsDir = path.resolve(process.env.GITHUB_WORKSPACE || '.', '.strata-reports');
    const blastPath = path.join(reportsDir, 'blast-radius.json');
    const hookPath = path.join(reportsDir, 'hook-prompt.json');

    let blastRadius = {
      analysis_status: ANALYSIS_UNAVAILABLE,
      analysis_error: 'blast-radius.json was missing or empty',
      modified_targets: [],
      total_impacted_nodes: 0,
      highest_risk_score: 0.0,
      breaking_risks_count: 0,
      triggered_anti_patterns: [],
      safe_to_apply: false,
      blast_reports: []
    };

    let hookPrompt = {
      known_failures: [],
      memories: [],
      query: ''
    };
    let hookError = '';

    if (fs.existsSync(blastPath)) {
      try {
        const rawBlast = fs.readFileSync(blastPath, 'utf8').trim();
        if (rawBlast) {
          blastRadius = JSON.parse(rawBlast);
        }
      } catch (err) {
        blastRadius.analysis_error = `failed to parse blast-radius.json: ${err.message}`;
        if (core) core.warning(`Failed parsing blast-radius.json: ${err.message}`);
      }
    }

    if (fs.existsSync(hookPath)) {
      try {
        const rawHook = fs.readFileSync(hookPath, 'utf8').trim();
        if (rawHook) {
          hookPrompt = JSON.parse(rawHook);
        } else {
          hookError = 'hook-prompt.json was empty';
        }
      } catch (err) {
        hookError = `failed to parse hook-prompt.json: ${err.message}`;
        if (core) core.warning(`Failed parsing hook-prompt.json: ${err.message}`);
      }
    } else {
      hookError = 'hook-prompt.json was missing';
    }

    let analysisStatus = normalizeAnalysisStatus(blastRadius.analysis_status);
    let analysisError = typeof blastRadius.analysis_error === 'string'
      ? blastRadius.analysis_error
      : '';

    if (hookPrompt.analysis_status === ANALYSIS_PARTIAL) {
      if (analysisStatus === ANALYSIS_COMPLETED) {
        analysisStatus = ANALYSIS_PARTIAL;
      }
      if (typeof hookPrompt.analysis_error === 'string' && hookPrompt.analysis_error) {
        analysisError = analysisError
          ? `${analysisError}; ${hookPrompt.analysis_error}`
          : hookPrompt.analysis_error;
      }
    }

    if (hookError) {
      if (analysisStatus === ANALYSIS_COMPLETED) {
        analysisStatus = ANALYSIS_PARTIAL;
      }
      analysisError = analysisError ? `${analysisError}; ${hookError}` : hookError;
    }

    blastRadius = {
      ...blastRadius,
      analysis_status: analysisStatus,
      analysis_error: analysisError
    };

    const prTitle = process.env.PR_TITLE || '';
    const report = formatReport(blastRadius, hookPrompt, prTitle);

    // 1. Set action outputs
    if (core) {
      core.setOutput('risk-level', report.riskLevel);
      core.setOutput('risk-score', report.riskPct.toString());
      core.setOutput('safe-to-apply', report.safeToApply.toString());
      core.setOutput('analysis-status', report.analysisStatus);
      core.setOutput('report-markdown', report.markdown);
      if (report.analysisStatus === ANALYSIS_FAILED || report.analysisStatus === ANALYSIS_UNAVAILABLE) {
        core.warning('Strata Guard analysis is incomplete; safe_to_apply is false.');
      }
    }

    // 2. Write to GITHUB_STEP_SUMMARY
    const stepSummaryPath = process.env.GITHUB_STEP_SUMMARY;
    if (stepSummaryPath) {
      fs.appendFileSync(stepSummaryPath, report.markdown + '\n\n', 'utf8');
      if (core) core.info('✓ Wrote report to GITHUB_STEP_SUMMARY');
    }

    // 3. Post or update sticky comment on PR
    const commentOnPr = process.env.COMMENT_ON_PR !== 'false';
    const prNumber = context?.payload?.pull_request?.number || context?.issue?.number;

    if (commentOnPr && prNumber && github && context?.repo) {
      const commentMarker = '<!-- strata-guard-report -->';
      try {
        const { data: comments } = await github.rest.issues.listComments({
          owner: context.repo.owner,
          repo: context.repo.repo,
          issue_number: prNumber,
          per_page: 100,
        });

        const existingComment = comments.find(c => c.body && c.body.includes(commentMarker));

        if (existingComment) {
          await github.rest.issues.updateComment({
            owner: context.repo.owner,
            repo: context.repo.repo,
            comment_id: existingComment.id,
            body: report.markdown,
          });
          if (core) core.info(`✓ Updated existing Strata Guard comment on PR #${prNumber} (ID: ${existingComment.id})`);
        } else {
          await github.rest.issues.createComment({
            owner: context.repo.owner,
            repo: context.repo.repo,
            issue_number: prNumber,
            body: report.markdown,
          });
          if (core) core.info(`✓ Created new Strata Guard comment on PR #${prNumber}`);
        }
      } catch (err) {
        if (core) {
          core.warning(`Could not post or update PR comment (permissions issue or fork): ${err.message}`);
        }
      }
    } else {
      if (core) core.info('ℹ️ Skipping PR comment (no PR context or comment-on-pr is disabled)');
    }

    // 4. Handle fail-on-critical
    const failOnCritical = process.env.FAIL_ON_CRITICAL === 'true';
    if (failOnCritical && report.riskLevel === 'Critical') {
      if (core) {
        core.setFailed(`🚨 Strata Guard failed: Pre-code risk is Critical (${report.riskPct}%).`);
      }
    }

    return report;
  } catch (globalErr) {
    if (core) {
      core.setFailed(`Strata Guard execution failed: ${globalErr.message}`);
    }
    throw globalErr;
  }
}

module.exports = runGuard;
module.exports.formatReport = formatReport;
module.exports.normalizeAnalysisStatus = normalizeAnalysisStatus;

// Allow standalone execution: node reviewer.js
if (require.main === module) {
  const dummyCore = {
    setOutput: (k, v) => console.log(`[OUTPUT] ${k}=${v}`),
    info: (m) => console.log(`[INFO] ${m}`),
    warning: (m) => console.warn(`[WARN] ${m}`),
    setFailed: (m) => { console.error(`[FAIL] ${m}`); process.exit(1); },
  };
  module.exports({ github: null, context: null, core: dummyCore }).catch(console.error);
}
