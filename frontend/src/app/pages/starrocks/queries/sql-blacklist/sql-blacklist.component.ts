import { Component, OnInit, OnDestroy, TemplateRef, ViewChild, ChangeDetectorRef } from '@angular/core';
import { NbToastrService, NbDialogService, NbDialogRef } from '@nebular/theme';
import { LocalDataSource } from 'angular2-smart-table';
import { Subject } from 'rxjs';
import { takeUntil } from 'rxjs/operators';
import { NodeService, SqlBlacklistItem } from '../../../../@core/data/node.service';
import { ClusterContextService } from '../../../../@core/data/cluster-context.service';
import { Cluster } from '../../../../@core/data/cluster.service';
import { ErrorHandler } from '../../../../@core/utils/error-handler';
import { ConfirmDialogService } from '../../../../@core/services/confirm-dialog.service';

@Component({
  standalone: false,
  selector: 'ngx-sql-blacklist',
  templateUrl: './sql-blacklist.component.html',
  styleUrls: ['./sql-blacklist.component.scss'],
})
export class SqlBlacklistComponent implements OnInit, OnDestroy {
  blacklistSource: LocalDataSource = new LocalDataSource();
  activeCluster: Cluster | null = null;
  loading = false;
  private destroy$ = new Subject<void>();

  @ViewChild('blacklistDialog', { static: false }) blacklistDialogTemplate!: TemplateRef<any>;
  blacklistDialogRef: NbDialogRef<any> | null = null;
  newBlacklistPattern = '';

  exampleTemplates = [
    {
      title: 'Block SELECT *',
      description: 'Prevent full table scans and improve query performance',
      pattern: 'select\\\\s+\\\\*\\\\s+from',
      icon: 'eye-off-outline'
    },
    {
      title: 'Block COUNT(*)',
      description: 'Avoid count operations on large tables',
      pattern: 'select\\\\s+count\\\\(\\\\*\\\\)\\\\s+from',
      icon: 'hash-outline'
    },
    {
      title: 'Block DROP operations',
      description: 'Prevent accidental deletion of tables, databases and views',
      pattern: '.*DROP\\\\s+(TABLE|DATABASE|VIEW).*',
      icon: 'trash-2-outline'
    },
    {
      title: 'Block unconditional DELETE',
      description: 'Prevent accidental deletion of all table data',
      pattern: 'DELETE\\\\s+FROM\\\\s+\\\\w+\\\\s*$',
      icon: 'alert-triangle-outline'
    },
    {
      title: 'Block queries on sensitive tables',
      description: 'Protect specific sensitive data tables',
      pattern: '.*FROM\\\\s+sensitive_table.*',
      icon: 'shield-outline'
    },
    {
      title: 'Block large bulk INSERT',
      description: 'Limit the number of rows inserted at once',
      pattern: 'INSERT\\\\s+INTO.*VALUES\\\\s*\\\\(.*\\\\)\\\\s*,.*\\\\)\\\\s*,.*\\\\)',
      icon: 'download-outline'
    }
  ];

  blacklistSettings = {
    mode: 'external',
    hideSubHeader: true,
    noDataMessage: 'No SQL blacklist rules',
    actions: { add: false, edit: false, delete: true, position: 'right' },
    delete: { deleteButtonContent: '<i class="nb-trash"></i>', confirmDelete: true },
    columns: {
      Id: { title: 'ID', type: 'string', width: '10%' },
      Pattern: { title: 'Regular Expression', type: 'string', width: '90%' },
    },
  };

  constructor(
    private nodeService: NodeService,
    private toastrService: NbToastrService,
    private clusterContext: ClusterContextService,
    private dialogService: NbDialogService,
    private confirmDialogService: ConfirmDialogService,
    private cdr: ChangeDetectorRef,
  ) {}

  ngOnInit(): void {
    this.clusterContext.activeCluster$.pipe(takeUntil(this.destroy$)).subscribe(cluster => {
      this.activeCluster = cluster;
      if (cluster) {
        this.loadBlacklistIfNotLoading();
      }
    });
  }

  private loadBlacklistIfNotLoading(): void {
    if (!this.loading) {
      this.loadBlacklist();
    }
  }

  ngOnDestroy(): void {
    this.destroy$.next();
    this.destroy$.complete();
  }

  loadBlacklist(): void {
    this.loading = true;
    this.nodeService.listSqlBlacklist().pipe(takeUntil(this.destroy$)).subscribe({
      next: items => {
        console.log('SQL Blacklist loaded:', items);
        console.log('Loading into table source...');
        this.blacklistSource.load(items);
        console.log('Table source count after load:', this.blacklistSource.count());
        this.loading = false;
      },
      error: error => {
        console.error('SQL Blacklist load error:', error);
        this.toastrService.danger(ErrorHandler.extractErrorMessage(error), 'Load failed');
        this.loading = false;
      },
    });
  }

  openAddDialog(): void {
    this.newBlacklistPattern = '';
    this.blacklistDialogRef = this.dialogService.open(this.blacklistDialogTemplate, { closeOnBackdropClick: false, closeOnEsc: true });
  }

  submitPattern(): void {
    const pattern = this.newBlacklistPattern.trim();
    if (!pattern) { this.toastrService.warning('Please enter a regular expression', 'Hint'); return; }
    this.nodeService.addSqlBlacklist(pattern).pipe(takeUntil(this.destroy$)).subscribe({
      next: () => {
        this.toastrService.success('SQL blacklist rule added', 'Success');
        this.blacklistDialogRef?.close();
        this.loadBlacklist();
      },
      error: error => { this.toastrService.danger(ErrorHandler.extractErrorMessage(error), 'Add failed'); },
    });
  }

  cancelDialog(): void { this.blacklistDialogRef?.close(); }

  useTemplate(template: any): void {
    this.newBlacklistPattern = template.pattern;
  }

  trackTemplate(index: number, template: any): string {
    return template.title;
  }

  onDeleteConfirm(event: any): void {
    const item = event.data;
    this.confirmDialogService.confirm('Confirm delete', `Are you sure you want to delete blacklist rule #${item.Id}?`, 'Delete', 'Cancel', 'danger').subscribe(confirmed => {
      if (!confirmed) { event.confirm.reject(); return; }
      this.nodeService.deleteSqlBlacklist(item.Id).pipe(takeUntil(this.destroy$)).subscribe({
        next: () => { this.toastrService.success('SQL blacklist rule deleted', 'Success'); this.loadBlacklist(); event.confirm.resolve(); },
        error: error => { this.toastrService.danger(ErrorHandler.extractErrorMessage(error), 'Delete failed'); event.confirm.reject(); },
      });
    });
  }
}
