import { Component, OnInit, OnDestroy } from '@angular/core';
import { Router } from '@angular/router';
import { NbToastrService } from '@nebular/theme';
import { LocalDataSource } from 'angular2-smart-table';
import { Subject } from 'rxjs';
import { takeUntil } from 'rxjs/operators';

import { ResourceGroupService } from '../resource-group.service';
import {
  ResourceUsageAnalysis,
  UserCpuUsage,
  UserMemoryUsage,
  UserConcurrency,
} from '../models/resource-group.model';

@Component({
  standalone: false,
  selector: 'ngx-resource-group-analysis',
  templateUrl: './resource-group-analysis.component.html',
  styleUrls: ['./resource-group-analysis.component.scss'],
})
export class ResourceGroupAnalysisComponent implements OnInit, OnDestroy {
  private destroy$ = new Subject<void>();

  loading = false;
  analysisData: ResourceUsageAnalysis | null = null;
  selectedDays = 30;

  cpuSource: LocalDataSource = new LocalDataSource();
  memorySource: LocalDataSource = new LocalDataSource();
  concurrencySource: LocalDataSource = new LocalDataSource();

  daysOptions = [
    { value: 7, label: 'Last 7 days' },
    { value: 30, label: 'Last 30 days' },
    { value: 90, label: 'Last 90 days' },
  ];

  cpuSettings = {
    actions: false,
    columns: {
      user: {
        title: 'User',
        type: 'string',
      },
      total_cpu_seconds: {
        title: 'Total CPU time (s)',
        type: 'number',
        valuePrepareFunction: (value: number) => value.toFixed(2),
      },
      cpu_usage_percentage: {
        title: 'CPU usage share (%)',
        type: 'number',
        valuePrepareFunction: (value: number) => value.toFixed(2) + '%',
      },
      suggested_cpu_weight: {
        title: 'Suggested CPU weight',
        type: 'number',
      },
      suggested_exclusive_cores: {
        title: 'Suggested exclusive cores',
        type: 'number',
      },
    },
  };

  memorySettings = {
    actions: false,
    columns: {
      user: {
        title: 'User',
        type: 'string',
      },
      max_mem_mb: {
        title: 'Max memory (MB)',
        type: 'number',
        valuePrepareFunction: (value: number) => value.toFixed(2),
      },
      suggested_mem_limit: {
        title: 'Suggested memory limit',
        type: 'string',
      },
      suggested_big_query_mem_limit: {
        title: 'Suggested big query memory limit',
        type: 'string',
      },
    },
  };

  concurrencySettings = {
    actions: false,
    columns: {
      user: {
        title: 'User',
        type: 'string',
      },
      max_concurrency_per_second: {
        title: 'Max concurrency (per second)',
        type: 'number',
        valuePrepareFunction: (value: number) => value.toFixed(2),
      },
      suggested_concurrency_limit: {
        title: 'Suggested concurrency limit',
        type: 'number',
      },
    },
  };

  constructor(
    private resourceGroupService: ResourceGroupService,
    private router: Router,
    private toastrService: NbToastrService,
  ) {}

  ngOnInit(): void {
    this.loadAnalysis();
  }

  ngOnDestroy(): void {
    this.destroy$.next();
    this.destroy$.complete();
  }

  loadAnalysis(): void {
    this.loading = true;
    this.resourceGroupService
      .getResourceUsageAnalysis(this.selectedDays)
      .pipe(takeUntil(this.destroy$))
      .subscribe({
        next: (analysis) => {
          this.analysisData = analysis;
          this.cpuSource.load(analysis.cpu_analysis);
          this.memorySource.load(analysis.memory_analysis);
          this.concurrencySource.load(analysis.concurrency_analysis);
          this.loading = false;
        },
        error: (error) => {
          console.error('Failed to load resource usage analysis:', error);
          this.toastrService.danger('Failed to load resource usage analysis', 'Error');
          this.loading = false;
        },
      });
  }

  onDaysChange(): void {
    this.loadAnalysis();
  }

  generateResourceGroupSQL(user: string): void {
    if (!this.analysisData) return;

    const cpuData = this.analysisData.cpu_analysis.find((c) => c.user === user);
    const memoryData = this.analysisData.memory_analysis.find((m) => m.user === user);
    const concurrencyData = this.analysisData.concurrency_analysis.find((c) => c.user === user);

    let sql = `CREATE RESOURCE GROUP ${user}_group\nTO (\n  user='${user}'\n)\nWITH (\n`;

    const withClauses = [];

    if (cpuData) {
      if (cpuData.suggested_exclusive_cores > 0) {
        withClauses.push(`  'exclusive_cpu_cores' = '${cpuData.suggested_exclusive_cores}'`);
      } else {
        withClauses.push(`  'cpu_weight' = '${cpuData.suggested_cpu_weight}'`);
      }
    }

    if (memoryData) {
      withClauses.push(`  'mem_limit' = '${memoryData.suggested_mem_limit}'`);
      withClauses.push(`  'big_query_mem_limit' = '${memoryData.suggested_big_query_mem_limit}'`);
    }

    if (concurrencyData) {
      withClauses.push(`  'concurrency_limit' = '${concurrencyData.suggested_concurrency_limit}'`);
    }

    sql += withClauses.join(',\n') + '\n);';

    // 复制到剪贴板
    navigator.clipboard.writeText(sql).then(() => {
      this.toastrService.success(`Resource group SQL for user ${user} copied to clipboard`, 'Success');
    });
  }

  goBack(): void {
    this.router.navigate(['/pages/cluster-ops/resource-groups']);
  }
}