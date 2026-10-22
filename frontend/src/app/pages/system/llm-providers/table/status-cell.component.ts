import { Component, Input } from '@angular/core';
import { LLMProvider } from '../../../../@core/data/llm-provider.service';

@Component({
  standalone: false,
  selector: 'ngx-llm-provider-status-cell',
  template: `
    <div class="d-flex flex-wrap align-items-center">
      <nb-badge
        [text]="rowData.is_active ? 'Active' : 'Inactive'"
        [status]="rowData.is_active ? 'success' : 'basic'"
        class="mr-1"
      ></nb-badge>
      <nb-badge
        [text]="rowData.enabled ? 'Enabled' : 'Disabled'"
        [status]="rowData.enabled ? 'info' : 'warning'"
      ></nb-badge>
    </div>
  `,
})
export class LLMProviderStatusCellComponent {
  @Input() rowData!: LLMProvider;
}
