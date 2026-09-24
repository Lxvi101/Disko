import React from 'react';
import { useStore } from '../../store';
import { api } from '../../lib/api';
import { FileTable } from '../FileTable';

export const FilesPage: React.FC = () => {
  const { currentItems, navigate, select, selectedPath, isStaged, toggleStage } = useStore();
  return (
    <div className="h-full px-6 pt-2 pb-4">
      <FileTable
        items={currentItems}
        onNavigate={navigate}
        onSelect={select}
        selectedPath={selectedPath}
        isStaged={isStaged}
        onToggleStage={(it) => toggleStage({ ...it, source: 'explore' })}
        onReveal={(p) => api.reveal(p).catch(() => {})}
      />
    </div>
  );
};
